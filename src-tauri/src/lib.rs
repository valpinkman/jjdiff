//! jjdiff application shell: launch options, app state, IPC commands, repo watchers.
//!
//! Commands that call jj or walk the filesystem are `async` and run their blocking work on
//! the runtime's blocking pool — sync Tauri commands execute on the main thread, and a slow
//! `jj` invocation there would freeze the window.

pub mod cli;
mod comments;
mod config;
mod editor;
mod forge;
mod menu;
mod viewed;
pub mod walkthrough;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{BookmarkStatus, Change, EvologEntry, Operation, Outcome, Repo};
use jjdiff_watch::RepoWatcher;
use viewed::ReviewStore;

use cli::Args;
use comments::{Comment, CommentStore, NewComment, Side};

/// `jjdiff [revset] [-R|--repo <path>] [-w|--walkthrough] [--walkthrough-file <path>]`
///
/// Built from the shared [`Args`] parser so the GUI and the headless CLI agree
/// on what a valid invocation looks like. Headless flags (`--help`, `--diff`,
/// …) are already dispatched in `main.rs` before `run` is reached; here we
/// only carry the GUI-relevant fields.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    pub repo_path: PathBuf,
    pub revset: Option<String>,
    /// Generate a walkthrough for the launch target immediately.
    pub walkthrough: bool,
    /// Agent-authored walkthrough JSON to import instead of generating one.
    pub walkthrough_file: Option<PathBuf>,
    /// `jjdiff pr 75` — open this forge proposal for review on launch.
    pub pull_request: Option<u32>,
}

impl LaunchOptions {
    /// Construct from an already-parsed [`Args`]. `main.rs` parses argv once
    /// and passes it here; headless commands have already been dispatched.
    pub fn from_args(args: &Args) -> LaunchOptions {
        let repo_path = args.repo_or_cwd();
        LaunchOptions {
            repo_path,
            revset: args.revset.clone(),
            walkthrough: args.walkthrough,
            walkthrough_file: args.walkthrough_file.clone(),
            pull_request: args.pull_request,
        }
    }
}

/// Everything one window owns. A window is bound to exactly one repository;
/// opening a different repo either reuses the window showing it or creates a
/// new one, so `Repo` and its watchers cannot be app-global.
struct WindowState {
    launch: LaunchOptions,
    repo: Option<Repo>,
    watchers: Vec<RepoWatcher>,
}

struct AppState {
    /// Launch options for the first window; later windows derive their own.
    launch: LaunchOptions,
    /// Per-window repo + watchers, keyed by Tauri window label.
    windows: Mutex<HashMap<String, WindowState>>,
    /// Source of the next window label. Labels must be unique for the lifetime
    /// of the process, so this only ever counts up.
    next_window: Mutex<u32>,
    // The review and comment stores stay app-global on purpose: both are keyed
    // by repo root, so two windows on the same repo must see the same comments
    // and the same viewed flags.
    review: Mutex<ReviewStore>,
    comments: Mutex<CommentStore>,
    recents: Mutex<Vec<String>>,
    recents_path: Mutex<Option<PathBuf>>,
}

/// The label of the window created at startup. Fixed so `tauri.conf.json` can
/// declare it and `setup` can find it.
const MAIN_WINDOW: &str = "main";

const MAX_RECENTS: usize = 8;

fn load_recents(path: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn remember_repo(state: &tauri::State<'_, AppState>, root: &str) {
    let mut recents = state.recents.lock().expect("recents lock");
    recents.retain(|entry| entry != root);
    recents.insert(0, root.to_string());
    recents.truncate(MAX_RECENTS);
    if let Some(path) = state.recents_path.lock().expect("recents path").as_ref() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&*recents) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// (Re)start both watchers for `repo` and title the window after it.
///
/// Events go to `label` alone (`emit_to`, not `emit`): a mutation in one repo's
/// window must not make every other window reload.
fn attach_repo(app: &AppHandle, label: &str, repo: &Repo) -> Vec<RepoWatcher> {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.set_title(&window_title(repo.root()));
    }
    let mut watchers = Vec::new();
    let notify = |app: &AppHandle, label: &str| {
        let handle = app.clone();
        let label = label.to_string();
        move || {
            let _ = handle.emit_to(&label, "repo-changed", ());
        }
    };
    match jjdiff_watch::watch_op_heads(
        &repo.op_heads_dir(),
        Duration::from_millis(250),
        notify(app, label),
    ) {
        Ok(watcher) => watchers.push(watcher),
        Err(error) => eprintln!("jjdiff: op watcher disabled: {error}"),
    }
    match jjdiff_watch::watch_working_copy(
        repo.root(),
        Duration::from_millis(400),
        notify(app, label),
    ) {
        Ok(watcher) => watchers.push(watcher),
        Err(error) => eprintln!("jjdiff: fs watcher disabled: {error}"),
    }
    watchers
}

fn window_title(root: &Path) -> String {
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    format!("jjdiff — {name}")
}

/// Bind `repo` to `label`, replacing whatever that window held before (the old
/// watchers drop with the old state, which is what stops them firing).
fn bind_window(app: &AppHandle, state: &AppState, label: &str, repo: Repo, launch: LaunchOptions) {
    let watchers = attach_repo(app, label, &repo);
    state.windows.lock().expect("windows lock").insert(
        label.to_string(),
        WindowState { launch, repo: Some(repo), watchers },
    );
}

/// The label of the window already showing `root`, if any.
fn window_for_repo(state: &AppState, root: &Path) -> Option<String> {
    state
        .windows
        .lock()
        .expect("windows lock")
        .iter()
        .find(|(_, window)| window.repo.as_ref().is_some_and(|repo| repo.root() == root))
        .map(|(label, _)| label.clone())
}

/// Serializable snapshot for the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepoState {
    root: PathBuf,
    jj_version: String,
    working_copy: Change,
    stack: Vec<Change>,
    /// Recent history (ancestors of @ and all bookmarks) for the graph view.
    graph: Vec<Change>,
    /// Ahead/behind for every bookmark tracking a remote. Empty on a repo with
    /// no remotes, which is not an error — it is most repos jjdiff opens.
    bookmarks: Vec<BookmarkStatus>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewStatus {
    viewed: Vec<String>,
    /// Commit id stored when the change was last marked reviewed.
    reviewed_commit: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Interdiff {
    files: Vec<FilePatch>,
    from_commit: String,
    to_commit: String,
}

/// Clone the (cheap) repo handle for the calling window, discovering it on
/// first use. Every repo-touching command resolves through here, which is what
/// keeps two windows on two repositories from reading each other's state.
fn repo_handle(state: &tauri::State<'_, AppState>, window: &tauri::Window) -> Result<Repo, String> {
    let label = window.label();
    let mut windows = state.windows.lock().expect("windows lock");
    let entry = windows
        .entry(label.to_string())
        .or_insert_with(|| WindowState {
            launch: state.launch.clone(),
            repo: None,
            watchers: Vec::new(),
        });
    if entry.repo.is_none() {
        let repo = Repo::discover(&entry.launch.repo_path).map_err(|e| e.to_string())?;
        // Fail loudly here rather than with a confusing template parse error later.
        repo.check_version().map_err(|e| e.to_string())?;
        entry.repo = Some(repo);
    }
    Ok(entry.repo.as_ref().expect("repo present").clone())
}

/// Run blocking jj/fs work off the main thread.
async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|e| e.to_string())?
}

fn vcs<T>(result: jjdiff_vcs::Result<T>) -> Result<T, String> {
    result.map_err(|e| e.to_string())
}

/// Launch options for the calling window. Windows opened later carry their own
/// repo path (and no `-w`/`--walkthrough-file`, which apply once at startup).
#[tauri::command]
fn launch_options(window: tauri::Window, state: tauri::State<'_, AppState>) -> LaunchOptions {
    state
        .windows
        .lock()
        .expect("windows lock")
        .get(window.label())
        .map(|entry| entry.launch.clone())
        .unwrap_or_else(|| state.launch.clone())
}

#[tauri::command]
fn get_config() -> config::Config {
    config::load()
}

/// Persist `[editor] command` so "Open in Editor" is configurable from inside
/// the app rather than only by hand-editing the config file. Returns the path
/// written, so the UI can say where the setting went.
#[tauri::command]
async fn set_editor_command(command: String) -> Result<String, String> {
    blocking(move || {
        config::set_editor_command(command.trim()).map(|path| path.display().to_string())
    })
    .await
}

/// Persist `[ui] theme`, so a palette picked in the app is still there next
/// launch. The name is not validated here — the frontend owns the theme list,
/// and an unknown one falls back to `system` on load rather than failing.
#[tauri::command]
async fn set_ui_theme(theme: String) -> Result<String, String> {
    blocking(move || config::set_ui_theme(theme.trim()).map(|path| path.display().to_string()))
        .await
}

/// Rebuild the native menu from the frontend's command list.
///
/// Only the focused window may set it: on macOS the menu bar is app-global, so
/// an unfocused window pushing its own commands would leave the menu describing
/// a repository the user is not looking at.
#[tauri::command]
fn set_menu(
    app: AppHandle,
    window: tauri::Window,
    groups: Vec<menu::MenuGroup>,
) -> Result<(), String> {
    if !window.is_focused().unwrap_or(true) {
        return Ok(());
    }
    let built = menu::build(&app, &groups).map_err(|error| error.to_string())?;
    app.set_menu(built).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
async fn repo_state(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
) -> Result<RepoState, String> {
    let repo = repo_handle(&state, &window)?;
    let graph_revset = revset.unwrap_or_else(|| "ancestors(@ | bookmarks())".to_string());
    blocking(move || {
        Ok(RepoState {
            root: repo.root().to_path_buf(),
            jj_version: vcs(repo.jj_version())?,
            working_copy: vcs(repo.working_copy())?,
            stack: vcs(repo.stack())?,
            graph: vcs(repo.graph(&graph_revset, 200))?,
            // Tolerated rather than propagated: a repo whose remote state cannot
            // be read is still perfectly reviewable, and failing the whole
            // repo_state call would black out the window over a badge.
            bookmarks: repo.bookmark_statuses().unwrap_or_default(),
        })
    })
    .await
}

/// Shared by `diff` and walkthrough generation. `revset: None` = live working copy
/// (fs-vs-`@-` through gix — no snapshot, no operation written); otherwise parses
/// `jj diff --git` output.
fn compute_diff(
    repo: &Repo,
    revset: Option<&str>,
    ignore_whitespace: bool,
) -> Result<Vec<FilePatch>, String> {
    match revset {
        Some(revset) => {
            let patch = vcs(repo.patch_for(revset, ignore_whitespace))?;
            jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())
        }
        None => {
            let base = vcs(repo.working_copy_parent())?;
            jjdiff_diff::worktree::diff_worktree(
                repo.root(),
                base.as_deref(),
                jjdiff_diff::worktree::WorktreeDiffOptions { ignore_whitespace },
            )
            .map_err(|e| e.to_string())
        }
    }
}

/// Structured diff for one revision — or the live working copy when `revset` is `None`.
#[tauri::command]
async fn diff(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<Vec<FilePatch>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || compute_diff(&repo, revset.as_deref(), ignore_whitespace)).await
}

/// Interdiff from the last-reviewed commit of `change_id` to its current commit.
#[tauri::command]
async fn interdiff_since_reviewed(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    ignore_whitespace: bool,
) -> Result<Interdiff, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    let from = state
        .review
        .lock()
        .expect("review lock")
        .reviewed_commit(&repo_key, &change_id)
        .ok_or_else(|| "change has no reviewed commit recorded".to_string())?;
    blocking(move || {
        let current = vcs(repo.log(&change_id))?
            .into_iter()
            .next()
            .ok_or_else(|| format!("change {change_id} not found"))?;
        let patch = vcs(repo.interdiff(&from, &current.commit_id, ignore_whitespace))?;
        Ok(Interdiff {
            files: jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())?,
            from_commit: from,
            to_commit: current.commit_id,
        })
    })
    .await
}

/// Every recorded version of `change_id`, newest first — jj's evolog. Entry 0 is the
/// change as it stands now; the rest are the commits it used to be.
#[tauri::command]
async fn change_versions(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<Vec<EvologEntry>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.evolog(&change_id))).await
}

/// Interdiff between two arbitrary versions of a change — the evolog drawer's payload.
/// Both commits may be hidden, which is exactly why they are addressed by commit id.
#[tauri::command]
async fn interdiff(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    from_commit: String,
    to_commit: String,
    ignore_whitespace: bool,
) -> Result<Interdiff, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || {
        let patch = vcs(repo.interdiff(&from_commit, &to_commit, ignore_whitespace))?;
        Ok(Interdiff {
            files: jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())?,
            from_commit,
            to_commit,
        })
    })
    .await
}

/// Tell every window bound to `root` that the repo moved. Scoped by repo rather
/// than by window: a mutation is visible to all windows showing that repo, and
/// to none of the others.
fn emit_repo_changed(app: &AppHandle, state: &AppState, root: &Path) {
    let labels: Vec<String> = state
        .windows
        .lock()
        .expect("windows lock")
        .iter()
        .filter(|(_, window)| window.repo.as_ref().is_some_and(|repo| repo.root() == root))
        .map(|(label, _)| label.clone())
        .collect();
    for label in labels {
        let _ = app.emit_to(&label, "repo-changed", ());
    }
}

/// Every mutation follows the same shape: run it off the main thread, then emit
/// `repo-changed` so the UI reloads. The returned [`Outcome`] carries jj's own narration
/// plus the operation id, which is what makes a one-click undo possible.
async fn run_mutation<F>(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    repo: Repo,
    task: F,
) -> Result<Outcome, String>
where
    F: FnOnce(&Repo) -> jjdiff_vcs::Result<Outcome> + Send + 'static,
{
    let root = repo.root().to_path_buf();
    let outcome = blocking(move || vcs(task(&repo))).await?;
    emit_repo_changed(app, state, &root);
    Ok(outcome)
}

#[tauri::command]
async fn describe(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    message: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, repo, move |repo| repo.describe(&change_id, &message)).await
}

#[tauri::command]
async fn new_change(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    parents: Vec<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.new_change(&parents)).await
}

/// `jj edit` — move the working copy onto an existing change.
#[tauri::command]
async fn edit_change(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, repo, move |repo| repo.edit(&revset)).await
}

/// Move working-copy `paths` into `into` (defaults to the parent): jj-native partial commit.
#[tauri::command]
async fn squash_paths(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
    into: Option<String>,
    from: Option<String>,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, repo, move |repo| {
        repo.squash_paths(
            from.as_deref().unwrap_or("@"),
            into.as_deref().unwrap_or("@-"),
            &paths,
        )
    })
    .await
}

#[tauri::command]
async fn absorb(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, |repo| repo.absorb()).await
}

/// File-level split: `paths` stay in the change, the rest move to a new child.
#[tauri::command]
async fn split_paths(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
    paths: Vec<String>,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    if paths.is_empty() {
        return Err("select at least one file to split out".into());
    }
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, repo, move |repo| repo.split_paths(&revset, &paths)).await
}

#[tauri::command]
async fn abandon_change(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, repo, move |repo| repo.abandon(&revset)).await
}

#[tauri::command]
async fn duplicate_change(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.duplicate(&revset)).await
}

#[tauri::command]
async fn backout_change(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.backout(&revset)).await
}

/// `mode` is "revision" (just it), "source" (it and descendants) or "branch".
#[tauri::command]
async fn rebase_change(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    mode: String,
    revset: String,
    destination: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, repo, move |repo| repo.rebase(&mode, &revset, &destination)).await
}

/// Discard working-copy changes to `paths` (all when empty). Undoable via the op log.
#[tauri::command]
async fn restore_paths(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.restore_paths(&paths)).await
}

#[tauri::command]
async fn set_bookmark(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    name: String,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.bookmark_set(&name, &revset)).await
}

#[tauri::command]
async fn delete_bookmark(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.bookmark_delete(&name)).await
}

#[tauri::command]
async fn git_fetch(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    remote: Option<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.git_fetch(remote.as_deref())).await
}

/// Push a bookmark, or `change` to auto-name one from the change id. The forge prints a
/// ready-made pull-request URL on a branch-creating push; [`pull_request_url`] finds it.
#[tauri::command]
async fn git_push(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    remote: Option<String>,
    bookmark: Option<String>,
    change: Option<String>,
) -> Result<PushResult, String> {
    let repo = repo_handle(&state, &window)?;
    let outcome = run_mutation(&app, &state, repo, move |repo| {
        repo.git_push(remote.as_deref(), bookmark.as_deref(), change.as_deref())
    })
    .await?;
    let url = pull_request_url(&outcome.message);
    Ok(PushResult { outcome, pull_request_url: url })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PushResult {
    #[serde(flatten)]
    outcome: Outcome,
    /// Forge-provided "create a pull request" link, when the push produced one.
    pull_request_url: Option<String>,
}

/// Scrape the pull-request URL forges print on a branch-creating push. Works for tangled,
/// GitHub, GitLab and Gitea without any forge API, auth, or per-forge code.
fn pull_request_url(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|token| {
            let token = token.trim_end_matches(['.', ',']);
            token.starts_with("http")
                && ["pulls/new", "pull/new", "merge_requests/new", "compare/", "/pulls?"]
                    .iter()
                    .any(|marker| token.contains(marker))
        })
        .map(|token| {
            token
                .trim_end_matches(['.', ','])
                .to_string()
        })
}

#[tauri::command]
async fn remotes(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.remotes())).await
}

// -- Operation log / undo --

#[tauri::command]
async fn operation_log(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    limit: usize,
) -> Result<Vec<Operation>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.operations(limit))).await
}

/// jj's own account of what an operation changed. `from` empty means "against its parent".
/// Returned as text: `jj op diff` has no `json()` form, so this is narration to display,
/// not output to parse.
#[tauri::command]
async fn operation_diff(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    from: Option<String>,
    to: String,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.op_diff(from.as_deref(), &to))).await
}

#[tauri::command]
async fn undo(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, |repo| repo.undo()).await
}

#[tauri::command]
async fn restore_operation(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    operation: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.op_restore(&operation)).await
}

#[tauri::command]
async fn revert_operation(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    operation: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| repo.op_revert(&operation)).await
}

#[tauri::command]
async fn conflicts(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Vec<String>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.conflicted_paths(&revset))).await
}

#[tauri::command]
fn review_status(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<ReviewStatus, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    let review = state.review.lock().expect("review lock");
    Ok(ReviewStatus {
        viewed: review.viewed(&repo_key, &change_id),
        reviewed_commit: review.reviewed_commit(&repo_key, &change_id),
    })
}

#[tauri::command]
fn set_viewed(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    path: String,
    viewed: bool,
) -> Result<(), String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    state
        .review
        .lock()
        .expect("review lock")
        .set_viewed(&repo_key, &change_id, &path, viewed);
    Ok(())
}

/// Import an agent-authored walkthrough for `change_id`: same validation as a generated
/// one (hunk ids checked against the real diff, files kept whole), then stored so it
/// behaves identically — staleness, stack review, everything.
#[tauri::command]
async fn import_walkthrough(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
    path: String,
) -> Result<walkthrough::Walkthrough, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    let imported = blocking(move || {
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {path}: {error}"))?;
        let files = compute_diff(&repo, revset.as_deref(), ignore_whitespace)?;
        walkthrough::parse_response(&raw, &files)
    })
    .await?;
    state
        .review
        .lock()
        .expect("review lock")
        .set_walkthrough(&repo_key, &change_id, imported.clone());
    Ok(imported)
}

/// Full text of a file, for expanding diff context. `revset: None` = the on-disk
/// working-copy version, matching what the live worktree diff shows.
#[tauri::command]
async fn file_content(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    path: String,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || match revset {
        Some(revset) => vcs(repo.file_content(&revset, &path)),
        None => {
            let full = repo.root().join(&path);
            std::fs::read_to_string(&full)
                .map_err(|error| format!("cannot read {}: {error}", full.display()))
        }
    })
    .await
}

/// Raw bytes of a file as base64 + mime type — for rendering images in the diff
/// view. `revset: None` = the on-disk working-copy version.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileBytes {
    /// Base64-encoded file contents.
    data: String,
    /// MIME type inferred from the extension (e.g. `image/png`).
    mime: String,
}

#[tauri::command]
async fn file_bytes(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    path: String,
) -> Result<FileBytes, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || {
        let bytes = match revset {
            Some(revset) => vcs(repo.file_bytes(&revset, &path))?,
            None => {
                let full = repo.root().join(&path);
                std::fs::read(&full)
                    .map_err(|error| format!("cannot read {}: {error}", full.display()))?
            }
        };
        // Size cap: 10 MB. Larger images are refused rather than base64-bloated into IPC.
        const MAX_BYTES: usize = 10 * 1024 * 1024;
        if bytes.len() > MAX_BYTES {
            return Err(format!(
                "file is {} KB — jjdiff only renders images up to {} KB",
                bytes.len() / 1024,
                MAX_BYTES / 1024
            ));
        }
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let mime = mime_for(&path);
        Ok(FileBytes { data, mime })
    })
    .await
}

/// Infer a MIME type from a file extension. Falls back to
/// `application/octet-stream` — the image view will refuse to render it.
fn mime_for(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Stored walkthrough for a change/// Stored walkthrough for a change, plus whether it still matches the current diff.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WalkthroughStatus {
    walkthrough: Option<walkthrough::Walkthrough>,
    stale: bool,
}

#[tauri::command]
async fn get_walkthrough(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<WalkthroughStatus, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    let stored = state.review.lock().expect("review lock").walkthrough(&repo_key, &change_id);
    let Some(stored) = stored else {
        return Ok(WalkthroughStatus { walkthrough: None, stale: false });
    };
    blocking(move || {
        let files = compute_diff(&repo, revset.as_deref(), ignore_whitespace)?;
        let stale = jjdiff_diff::diff_fingerprint(&files) != stored.fingerprint;
        Ok(WalkthroughStatus { walkthrough: Some(stored), stale })
    })
    .await
}

/// Generate (or regenerate) a walkthrough for `change_id` via the Claude CLI and store it.
#[tauri::command]
async fn generate_walkthrough(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
    context: String,
) -> Result<walkthrough::Walkthrough, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    let cfg = config::load().walkthrough;
    let generated = blocking(move || {
        let files = compute_diff(&repo, revset.as_deref(), ignore_whitespace)?;
        let selected = walkthrough::Backend::parse(&cfg.backend);
        let model = cfg.model_for(selected);
        let backend = walkthrough::CliBackend {
            backend: selected,
            model: (!model.is_empty()).then_some(model),
        };
        walkthrough::generate(&backend, &files, &context, &cfg.prompt)
    })
    .await?;
    state
        .review
        .lock()
        .expect("review lock")
        .set_walkthrough(&repo_key, &change_id, generated.clone());
    Ok(generated)
}

/// Discover and version-check a repo off the main thread.
async fn discover(path: String) -> Result<Repo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo = Repo::discover(Path::new(&path)).map_err(|e| e.to_string())?;
        repo.check_version().map_err(|e| e.to_string())?;
        Ok::<_, String>(repo)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Point the calling window at another repository (must be a colocated jj repo).
#[tauri::command]
async fn open_repository(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let repo = discover(path).await?;
    remember_repo(&state, &repo.root().to_string_lossy());
    let launch = LaunchOptions {
        repo_path: repo.root().to_path_buf(),
        revset: None,
        walkthrough: false,
        walkthrough_file: None,
        pull_request: None,
    };
    let root = repo.root().to_path_buf();
    bind_window(&app, &state, window.label(), repo, launch);
    emit_repo_changed(&app, &state, &root);
    Ok(())
}

/// Open `path` in its own window — or focus the window already showing it,
/// since two windows on one repo would just be two views of the same state.
#[tauri::command]
async fn open_repo_window(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let repo = discover(path).await?;
    if let Some(existing) = window_for_repo(&state, repo.root()) {
        if let Some(window) = app.get_webview_window(&existing) {
            let _ = window.set_focus();
            return Ok(());
        }
    }
    remember_repo(&state, &repo.root().to_string_lossy());
    let launch = LaunchOptions {
        repo_path: repo.root().to_path_buf(),
        revset: None,
        walkthrough: false,
        walkthrough_file: None,
        pull_request: None,
    };
    spawn_window(&app, &state, repo, launch)
}

/// Create a window and bind `repo` to it. The state entry is written *before*
/// the window is built, so the first `launch_options` call from the new
/// webview already finds its repo rather than falling back to the app default.
fn spawn_window(
    app: &AppHandle,
    state: &tauri::State<'_, AppState>,
    repo: Repo,
    launch: LaunchOptions,
) -> Result<(), String> {
    let label = {
        let mut next = state.next_window.lock().expect("window counter");
        *next += 1;
        format!("repo-{next}")
    };
    let title = window_title(repo.root());
    state.windows.lock().expect("windows lock").insert(
        label.clone(),
        WindowState { launch, repo: Some(repo.clone()), watchers: Vec::new() },
    );

    // `mut` is only used on macOS, below.
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::default())
        .title(title)
        .inner_size(1280.0, 840.0)
        // Matches `main` in tauri.conf.json. 1024 is the narrowest the layout is
        // designed for — a 52px rail plus a 292px sidebar leaves the diff pane
        // under 700px below that, which is not enough for a side-by-side diff.
        .min_inner_size(1024.0, 640.0);

    // Same chrome as the `main` window in tauri.conf.json: the title bar is an
    // overlay over the app's own background and the title is hidden, so the
    // traffic lights float on the page instead of sitting in a grey strip. The
    // title is still *set* — it is what the Window menu and Mission Control
    // show — it is only not drawn in the bar.
    //
    // Both builder methods are `#[cfg(target_os = "macos")]` in Tauri, so this
    // has to be gated: naming them unconditionally does not fail to *link* on
    // Linux, it fails to compile. The equivalent keys in tauri.conf.json are
    // cross-platform and simply ignored elsewhere, which is why only this call
    // needed the cfg.
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true);
    }

    builder.build().map_err(|error| {
        state.windows.lock().expect("windows lock").remove(&label);
        format!("cannot open a window: {error}")
    })?;

    // Watchers need the window to exist (they emit to its label).
    let watchers = attach_repo(app, &label, &repo);
    if let Some(entry) = state.windows.lock().expect("windows lock").get_mut(&label) {
        entry.watchers = watchers;
    }
    Ok(())
}

/// Native folder picker; returns the chosen path (not yet opened) or None on cancel.
#[tauri::command]
async fn pick_repository(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let picked = tauri::async_runtime::spawn_blocking(move || rx.recv().ok().flatten())
        .await
        .map_err(|e| e.to_string())?;
    Ok(picked.map(|p| p.to_string()))
}

#[tauri::command]
fn recent_repos(state: tauri::State<'_, AppState>) -> Vec<String> {
    state.recents.lock().expect("recents lock").clone()
}

/// Open `path` (repo-relative) in the configured editor, optionally at `line`.
/// Runs off the main thread: spawning is fast, but a cold editor binary on a
/// slow disk should not be able to stall the window.
#[tauri::command]
async fn open_in_editor(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    path: String,
    line: Option<u32>,
) -> Result<(), String> {
    let repo = repo_handle(&state, &window)?;
    let template = config::load().editor.command;
    blocking(move || {
        let root = repo.root();
        let argv = editor::build_argv(&template, &root.join(&path), line, root)?;
        editor::spawn(&argv)
    })
    .await
}

// -- Forge review (gh) --

/// Build a forge client for `repo` from its remote URL.
///
/// `origin` wins when present; otherwise the first remote does, because a repo
/// with exactly one differently-named remote is still unambiguous. A host we
/// cannot place is an error rather than a guess — being wrong here means
/// shelling out to a CLI that is not there.
fn forge_client(repo: &Repo) -> Result<forge::Client, String> {
    let remotes = vcs(repo.remote_urls())?;
    if remotes.is_empty() {
        return Err("this repository has no git remote, so there is nothing to review".into());
    }
    let (name, url) = remotes
        .iter()
        .find(|(name, _)| name == "origin")
        .or_else(|| remotes.first())
        .expect("remotes is non-empty");
    let kind = forge::Kind::from_remote(url).ok_or_else(|| {
        format!("jjdiff can't tell what forge `{name}` ({url}) is — only GitHub has a \
                 CLI it knows how to drive")
    })?;
    Ok(forge::Client { kind, root: repo.root().to_path_buf() })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForgeInfo {
    kind: forge::Kind,
    /// What the forge calls a proposal, for user-facing strings.
    noun: &'static str,
}

/// What forge this repo is on, or `None` when it is on none we can drive.
/// Deliberately not an error: the UI hides forge affordances rather than
/// showing a broken one.
#[tauri::command]
async fn forge_info(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Option<ForgeInfo>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || {
        Ok(forge_client(&repo)
            .ok()
            .map(|client| ForgeInfo { kind: client.kind, noun: client.kind.noun() }))
    })
    .await
}

#[tauri::command]
async fn list_pull_requests(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<forge::Summary>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.list(limit)).await
}

#[tauri::command]
async fn pull_request(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    number: u32,
) -> Result<forge::PullRequest, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.pull_request(number)).await
}

/// The proposal's conversation. Separate from [`pull_request`] on purpose: it
/// costs two more `gh` calls, and the banner should not wait on them to say
/// what the state and checks are.
#[tauri::command]
async fn pull_request_activity(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    number: u32,
) -> Result<Vec<forge::Activity>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.activity(number)).await
}

/// A fetched proposal: its metadata, plus the local bookmark its head landed on
/// so the UI can select it like any other change.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenedPullRequest {
    #[serde(flatten)]
    pull_request: forge::PullRequest,
    /// Revset for the proposal head.
    bookmark: String,
    /// Revset for just the proposal's own commits (`base..head`).
    revset: String,
}

/// Fetch a proposal's head and return everything needed to review it.
///
/// The head lands on a namespaced bookmark, which makes the whole thing
/// jj-native: from here on a pull request is just a revset, reviewed by the
/// same diff pane, walkthroughs and comments as anything else.
#[tauri::command]
async fn open_pull_request(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    number: u32,
) -> Result<OpenedPullRequest, String> {
    let repo = repo_handle(&state, &window)?;
    let root = repo.root().to_path_buf();
    let opened = blocking(move || {
        let client = forge_client(&repo)?;
        let pull_request = client.pull_request(number)?;
        let remotes = vcs(repo.remote_urls())?;
        let remote = remotes
            .iter()
            .find(|(name, _)| name == "origin")
            .or_else(|| remotes.first())
            .map(|(name, _)| name.clone())
            .ok_or_else(|| "this repository has no git remote".to_string())?;
        let bookmark = vcs(repo.fetch_forge_ref(
            &remote,
            &client.kind.head_ref(number),
            &client.kind.local_bookmark(number),
        ))?;
        // The merge-base commit is an ancestor of the base branch, so the base
        // branch has to be local for the review revset to resolve. Not fatal:
        // an already-current repo makes this a no-op, and an offline one still
        // gets a review if it happens to have the commit.
        if let Err(error) = repo.git_fetch_branch(&remote, &pull_request.base) {
            eprintln!("jjdiff: could not refresh {}: {error}", pull_request.base);
        }
        // Diff against the forge's own merge base rather than `base..head`,
        // which goes empty the moment a proposal is merged.
        let revset = if pull_request.base_oid.is_empty() {
            format!("{}..{bookmark}", pull_request.base)
        } else {
            format!("{}..{bookmark}", pull_request.base_oid)
        };
        Ok(OpenedPullRequest { pull_request, bookmark, revset })
    })
    .await?;
    // The fetch created a bookmark, so the graph changed.
    emit_repo_changed(&app, &state, &root);
    Ok(opened)
}

/// Submit a review. Outward-facing and effectively irreversible — the UI
/// confirms, naming the verdict, before this is reached.
#[tauri::command]
async fn submit_review(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    number: u32,
    verdict: forge::Verdict,
    body: String,
    comments: Vec<forge::ReviewComment>,
) -> Result<forge::Submitted, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.submit_review(number, verdict, &body, &comments)).await
}

/// Open a URL in the system browser — the WebView cannot (see `editor::open_url`).
#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    blocking(move || editor::open_url(&url)).await
}

/// Write the `jjdiff` shim on PATH so the bundle is reachable from a shell.
/// Same logic as the headless `--install-terminal-helper` command; exposed
/// here so the in-app command bar can offer it too. Returns a human-readable
/// report (the path installed, or the command the user should run manually).
#[tauri::command]
fn install_terminal_helper() -> Result<String, String> {
    cli::install_terminal_helper()
}

/// Record `commit_id` as the reviewed baseline for `change_id`.
#[tauri::command]
fn mark_reviewed(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    commit_id: String,
) -> Result<(), String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    state
        .review
        .lock()
        .expect("review lock")
        .mark_reviewed(&repo_key, &change_id, &commit_id);
    Ok(())
}

// -- Inline review comments --

/// Add a comment anchored to a line in a change's diff. The anchor is keyed by
/// change id (not commit id), so it survives `describe`/`squash`/rebase.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn add_comment(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    path: String,
    hunk_id: String,
    side: Side,
    line: u32,
    line_text: String,
    commit_id: String,
    author: String,
    body: String,
    parent_id: Option<i64>,
) -> Result<Comment, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    state.comments.lock().expect("comments lock").add(NewComment {
        repo: repo_key,
        change_id,
        path,
        hunk_id,
        side,
        line,
        line_text,
        commit_id,
        author,
        body,
        parent_id,
    })
}

/// All comments for a change, ordered by path then line then time.
#[tauri::command]
fn list_comments(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<Vec<Comment>, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    state.comments.lock().expect("comments lock").list(&repo_key, &change_id)
}

/// Mark a comment resolved (or unresolved).
#[tauri::command]
fn set_comment_resolved(
    state: tauri::State<'_, AppState>,
    id: i64,
    resolved: bool,
) -> Result<(), String> {
    state
        .comments
        .lock()
        .expect("comments lock")
        .set_resolved(id, resolved)
}

/// Delete a comment and its children.
#[tauri::command]
fn delete_comment(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    state.comments.lock().expect("comments lock").delete(id)
}

/// Edit the body of a comment.
#[tauri::command]
fn update_comment(
    state: tauri::State<'_, AppState>,
    id: i64,
    body: String,
) -> Result<(), String> {
    state
        .comments
        .lock()
        .expect("comments lock")
        .update_body(id, &body)
}

/// Re-anchor comments for a change against the current diff. Called by the UI
/// when a change evolves (commit id differs from what comments were written
/// against). Returns the number of comments whose anchor changed.
#[tauri::command]
async fn refresh_comment_anchors(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    current_commit_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<usize, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    // Compute the diff off the main thread, then run the re-anchoring (an
    // in-memory SQLite query) back on the caller. The comment store is not
    // `Send` through `blocking` because it borrows `state`.
    let files = blocking(move || compute_diff(&repo, revset.as_deref(), ignore_whitespace)).await?;
    state
        .comments
        .lock()
        .expect("comments lock")
        .refresh_anchors(&repo_key, &change_id, &current_commit_id, &files)
}

/// Render pending (unresolved) comments as a paste-ready Markdown review.
#[tauri::command]
fn export_review_markdown(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    state
        .comments
        .lock()
        .expect("comments lock")
        .export_markdown(&repo_key, &change_id)
}

pub fn run(args: Args) {
    let launch = LaunchOptions::from_args(&args);
    // SQLite wants a connection to open lazily after the data dir is known, so
    // the store starts empty and is opened in `setup` once `app_data_dir` is
    // available.
    let state = AppState {
        launch,
        windows: Mutex::new(HashMap::new()),
        next_window: Mutex::new(0),
        review: Mutex::new(ReviewStore::default()),
        comments: Mutex::new(CommentStore::in_memory().expect("comment db bootstrap")),
        recents: Mutex::new(Vec::new()),
        recents_path: Mutex::new(None),
    };

    let mut builder = tauri::Builder::default();

    // Single instance: launching `jjdiff` from a second repo opens a window in
    // the existing process rather than a rival process fighting over the same
    // review store. We parse the second instance's argv with the same [`Args`]
    // parser, then route it: a repo that already has a window gets that window
    // focused, anything else gets a new one.
    builder = builder.plugin(
        tauri_plugin_single_instance::init(|app, argv, cwd| {
            // argv[0] is the binary; the rest mirrors `jjdiff [flags] [revset]`.
            let rest: Vec<String> = argv.iter().skip(1).cloned().collect();
            let parsed = match Args::parse(&rest) {
                Ok(parsed) => parsed,
                Err(error) => {
                    eprintln!("jjdiff: ignoring second instance with bad args: {error}");
                    return;
                }
            };
            // The second invocation's cwd, not ours: `-R` is optional and the
            // bare `jjdiff` form means "the repo I am standing in".
            let target = parsed.repo_path.clone().unwrap_or_else(|| PathBuf::from(&cwd));
            let state = app.state::<AppState>();
            match Repo::discover(&target) {
                Ok(repo) => {
                    if let Some(label) = window_for_repo(&state, repo.root()) {
                        if let Some(window) = app.get_webview_window(&label) {
                            let _ = window.set_focus();
                            // Revset/walkthrough flags still apply to that window.
                            let _ = app.emit_to(&label, "second-instance", parsed);
                            return;
                        }
                    }
                    let launch = LaunchOptions {
                        repo_path: repo.root().to_path_buf(),
                        revset: parsed.revset.clone(),
                        walkthrough: parsed.walkthrough,
                        walkthrough_file: parsed.walkthrough_file.clone(),
                        pull_request: parsed.pull_request,
                    };
                    remember_repo(&state, &repo.root().to_string_lossy());
                    if let Err(error) = spawn_window(app, &state, repo, launch) {
                        eprintln!("jjdiff: {error}");
                    }
                }
                Err(error) => eprintln!("jjdiff: second instance: {error}"),
            }
        }),
    );

    builder
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        // A menu click carries only the command id; the frontend owns what it
        // does. Emitted app-wide and filtered there by document focus — with a
        // single app-global menu bar, exactly one window is ever the target.
        .on_menu_event(|app, event| {
            if let Some(id) = event.id().0.strip_prefix(menu::PREFIX) {
                let _ = app.emit("menu-command", id.to_string());
            }
        })
        .invoke_handler(tauri::generate_handler![
            launch_options,
            get_config,
            set_editor_command,
            set_ui_theme,
            set_menu,
            repo_state,
            diff,
            interdiff_since_reviewed,
            interdiff,
            change_versions,
            describe,
            new_change,
            squash_paths,
            absorb,
            conflicts,
            review_status,
            set_viewed,
            mark_reviewed,
            get_walkthrough,
            generate_walkthrough,
            open_repository,
            open_repo_window,
            pick_repository,
            recent_repos,
            install_terminal_helper,
            open_in_editor,
            open_url,
            forge_info,
            list_pull_requests,
            pull_request,
            pull_request_activity,
            open_pull_request,
            submit_review,
            file_content,
            file_bytes,
            import_walkthrough,
            edit_change,
            split_paths,
            abandon_change,
            duplicate_change,
            backout_change,
            rebase_change,
            restore_paths,
            set_bookmark,
            delete_bookmark,
            git_fetch,
            git_push,
            remotes,
            operation_log,
            operation_diff,
            undo,
            restore_operation,
            revert_operation,
            add_comment,
            list_comments,
            set_comment_resolved,
            delete_comment,
            update_comment,
            refresh_comment_anchors,
            export_review_markdown
        ])
        .setup(|app| {
            let state = app.state::<AppState>();

            let data_dir = app.path().app_data_dir().ok();
            let review_path = data_dir
                .as_ref()
                .map(|dir| dir.join("review.json"))
                .unwrap_or_else(|| PathBuf::from(".jjdiff-review.json"));
            *state.review.lock().expect("review lock") = ReviewStore::load(review_path);
            if let Some(dir) = data_dir {
                let recents_path = dir.join("recents.json");
                *state.recents.lock().expect("recents lock") = load_recents(&recents_path);
                *state.recents_path.lock().expect("recents path") = Some(recents_path);

                // Open the comment DB from the app data dir; fall back to
                // in-memory (already set in `run`) if it fails so the app
                // still starts — comments just won't persist across launches.
                if let Ok(db) = CommentStore::open(dir.join("comments.db")) {
                    *state.comments.lock().expect("comments lock") = db;
                } else {
                    eprintln!("jjdiff: comment db disabled (could not open comments.db)");
                }
            }

            // Bind the startup window. Watchers are not fatal: without them the
            // UI still works, it just won't live-refresh.
            let launch = state.launch.clone();
            if let Ok(repo) = Repo::discover(&launch.repo_path) {
                remember_repo(&state, &repo.root().to_string_lossy());
                bind_window(app.handle(), &state, MAIN_WINDOW, repo, launch);
            }
            Ok(())
        })
        // Drop a window's repo and watchers when it closes, so a long session
        // that opens and closes repos does not keep every one of them watched.
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .state::<AppState>()
                    .windows
                    .lock()
                    .expect("windows lock")
                    .remove(window.label());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running jjdiff");
}


#[cfg(test)]
mod tests {
    use super::pull_request_url;

    #[test]
    fn scrapes_forge_pull_request_urls() {
        // tangled (verbatim from a real push)
        let tangled = "remote: →  Open pull request:\nremote:    https://tangled.org/valpinkman.tngl.sh/jjdiff/pulls/new?source=branch&sourceBranch=x&targetBranch=main";
        assert_eq!(
            pull_request_url(tangled).as_deref(),
            Some("https://tangled.org/valpinkman.tngl.sh/jjdiff/pulls/new?source=branch&sourceBranch=x&targetBranch=main")
        );

        let github = "remote: Create a pull request for 'feature' on GitHub by visiting:\nremote:   https://github.com/owner/repo/pull/new/feature";
        assert_eq!(
            pull_request_url(github).as_deref(),
            Some("https://github.com/owner/repo/pull/new/feature")
        );

        let gitlab = "remote: To create a merge request for feature, visit:\nremote:   https://gitlab.com/owner/repo/-/merge_requests/new?merge_request%5Bsource_branch%5D=feature";
        assert!(pull_request_url(gitlab).unwrap().contains("merge_requests/new"));

        // Trailing punctuation must not end up in the URL.
        let punctuated = "see https://github.com/o/r/pull/new/x.";
        assert_eq!(
            pull_request_url(punctuated).as_deref(),
            Some("https://github.com/o/r/pull/new/x")
        );
    }

    #[test]
    fn ordinary_push_output_has_no_url() {
        assert!(pull_request_url("bookmark: main [move forward from a to b]").is_none());
        assert!(pull_request_url("Nothing changed.").is_none());
    }
}
