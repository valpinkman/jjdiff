//! jjdiff application shell: launch options, app state, IPC commands, repo watchers.
//!
//! Commands that call jj or walk the filesystem are `async` and run their blocking work on
//! the runtime's blocking pool — sync Tauri commands execute on the main thread, and a slow
//! `jj` invocation there would freeze the window.

pub mod cli;
mod comments;
mod config;
mod describe;
mod editor;
mod forge;
mod menu;
mod resolve;
mod split;
mod viewed;
mod workspaces;
pub mod walkthrough;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{
    BookmarkStatus, Change, ConflictedFile, EvologEntry, Operation, Outcome, Repo, Workspace,
};
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
        let repo_path = args.workspace_or_repo();
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

/// Add a repository to the Open Recent list.
///
/// Callers pass a [`review_key`] — the *repository*, never a workspace path. A workspace is
/// a second checkout of somewhere already on this list, so recording one both duplicates an
/// entry and pushes a real repo off the end. Opening a workspace therefore remembers the
/// repo it belongs to, and the Trees pane is where its own directories are listed.
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
        let _ = window.set_title(&window_title(repo));
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

/// `jjdiff — codiff`, or `jjdiff — codiff [build]` for a workspace that is not the repo's
/// own directory.
///
/// The suffix is derived from the paths rather than asked of jj, because titling a window
/// should not cost a subprocess — and because the question it answers is "is this the tree I
/// think it is", which the directory names answer directly. A repo with one workspace is
/// titled exactly as before.
fn window_title(repo: &Repo) -> String {
    let name_of = |path: &Path| path.file_name().map(|n| n.to_string_lossy().into_owned());
    let repo_name = name_of(&repo.review_key()).unwrap_or_default();
    match name_of(repo.root()) {
        Some(workspace) if workspace != repo_name => format!("jjdiff — {repo_name} [{workspace}]"),
        _ => format!("jjdiff — {repo_name}"),
    }
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
    /// The *repository's* name, which is the workspace's only until a second workspace
    /// exists. `root`'s basename is the workspace directory, so a header built from it would
    /// call the repo `build` while showing `build`'s contents — the one place the two names
    /// differ is exactly the place it matters.
    repo_name: String,
    jj_version: String,
    working_copy: Change,
    stack: Vec<Change>,
    /// Recent history (ancestors of @ and all bookmarks) for the graph view.
    graph: Vec<Change>,
    /// Ahead/behind for every bookmark tracking a remote. Empty on a repo with
    /// no remotes, which is not an error — it is most repos jjdiff opens.
    bookmarks: Vec<BookmarkStatus>,
    /// Commit ids of work that is on no remote — the other half of `bookmarks`.
    /// A change with no bookmark tracks nothing, so it can never appear there
    /// however long it goes unpushed; this is what makes it visible. Empty when
    /// the repo has no remote at all, where the question means nothing.
    /// Commits rather than changes: see [`Repo::unpushed`].
    unpushed: Vec<String>,
    /// Every workspace attached to this repo, this one included. Always at least one, so
    /// the pane can tell "one workspace" from "not loaded yet".
    workspaces: Vec<WorkspaceView>,
    /// Which of them this window is showing. `None` only if jj and jjdiff disagree about
    /// the root, which should not happen and is not worth failing a refresh over.
    workspace: Option<String>,
}

/// A workspace as the UI sees it: jj's facts, plus the one thing only jjdiff knows.
///
/// `generated` decides whether the UI offers to *delete* a workspace's files or only to
/// forget it. It is computed here rather than in the frontend because the rule depends on
/// the configured root, and a second copy of that rule in TypeScript would be a second
/// chance to get "may I remove this directory" wrong. The backend enforces it again on the
/// way in regardless — this flag shapes the affordance, `forget_workspace` is the guarantee.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceView {
    #[serde(flatten)]
    workspace: Workspace,
    generated: bool,
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

/// The key review state is filed under.
///
/// Every workspace of a repository shares it, which is the point: viewed flags, comments,
/// "last reviewed" and walkthroughs are keyed by change id so they survive the change being
/// rewritten, and a change checked out in a second workspace is the same change. Filing them
/// under the tree it happens to sit in would undo that for no gain.
///
/// It is the workspace root itself in a repo with one workspace, so nothing anyone has
/// already stored changes meaning.
fn review_key(repo: &Repo) -> String {
    repo.review_key().to_string_lossy().into_owned()
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

/// The repo half of every review-state key: viewed flags, reviewed commits,
/// walkthroughs (`ReviewStore`) and comments (`CommentStore`) are all stored
/// under the string form of the window's **repository**.
///
/// One function because the format is a stored contract — a site that spelled
/// it differently, by canonicalising the path or by using the launch path
/// (which may be a subdirectory `Repo::discover` resolves via `jj root`), would
/// key that user's review state under a name nothing else reads, and their
/// notes and viewed flags would simply be gone.
///
/// The repository, not the workspace: [`Repo::review_key`] resolves the directory holding
/// the shared `.jj`, so a change reviewed in one workspace keeps its comments and viewed
/// flags when it is checked out in another — which is the whole point of keying review state
/// on change ids. In a repo with one workspace it *is* the root, byte for byte, so nothing
/// stored under the old spelling moved.
///
/// It reads that off the repo already bound to the window, so a command
/// that needs nothing else runs no subprocess; only a window whose repo has not
/// been discovered yet falls through to [`repo_handle`].
fn repo_key(state: &tauri::State<'_, AppState>, window: &tauri::Window) -> Result<String, String> {
    let bound = state
        .windows
        .lock()
        .expect("windows lock")
        .get(window.label())
        .and_then(|entry| entry.repo.as_ref())
        .map(review_key);
    match bound {
        Some(key) => Ok(key),
        None => Ok(review_key(&repo_handle(state, window)?)),
    }
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

/// Persist one setting from the settings page. The table and key must be on
/// [`config`]'s allow-list and the value must match its declared type; both are
/// refused rather than written, and the error says which.
#[tauri::command]
async fn set_config_value(
    table: String,
    key: String,
    value: serde_json::Value,
) -> Result<String, String> {
    blocking(move || {
        config::set_setting(&table, &key, &value).map(|path| path.display().to_string())
    })
    .await
}

/// Where the settings live, so the page can name the file it is editing — and
/// so "edit it by hand" stays an obvious option rather than a secret.
#[tauri::command]
fn config_file_path() -> String {
    config::config_path().map(|path| path.display().to_string()).unwrap_or_default()
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
        // Tolerated rather than propagated, for the same reason as bookmarks below: a
        // repo whose workspace list cannot be read is still perfectly reviewable.
        let generated_root = config::load().workspace.resolved_root();
        let listed = repo.workspaces().unwrap_or_default();
        let workspace = listed.iter().find(|w| w.current).map(|w| w.name.clone());
        let workspaces: Vec<WorkspaceView> = listed
            .into_iter()
            .map(|workspace| WorkspaceView {
                generated: workspace.path.as_deref().is_some_and(|path| {
                    workspaces::is_generated(generated_root.as_deref(), Path::new(path))
                }),
                workspace,
            })
            .collect();
        Ok(RepoState {
            root: repo.root().to_path_buf(),
            repo_name: repo
                .review_key()
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            jj_version: vcs(repo.jj_version())?,
            working_copy: vcs(repo.working_copy())?,
            stack: vcs(repo.stack())?,
            graph: vcs(repo.graph(&graph_revset, 200))?,
            // Tolerated rather than propagated: a repo whose remote state cannot
            // be read is still perfectly reviewable, and failing the whole
            // repo_state call would black out the window over a badge.
            bookmarks: repo.bookmark_statuses().unwrap_or_default(),
            // Same tolerance, same reason: this drives a badge.
            unpushed: repo.unpushed().unwrap_or_default(),
            workspaces,
            workspace,
        })
    })
    .await
}

/// Shared by `diff`, walkthrough generation and the headless commands in `cli`.
/// `revset: None` = live working copy
/// (fs-vs-`@-` through gix — no snapshot, no operation written); otherwise parses
/// `jj diff --git` output.
pub(crate) fn compute_diff(
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
                repo.git_dir(),
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

/// Interdiff from the last-reviewed commit of `change_id` to `to_commit`.
///
/// Both ids, because this is the one command that needs both: `change_id` is the
/// **key** the reviewed baseline is filed under, and `to_commit` is the
/// **revision** to diff it against. It used to take only the change id and
/// resolve the second with `repo.log(&change_id)`, which is the pattern that
/// broke on a divergent change — one change id over several visible commits,
/// which jj refuses to resolve at all. The caller is the only party that knows
/// which of them is on screen, so it says.
///
/// Dropping the `log` also drops a subprocess: the frontend already had the
/// commit in hand.
#[tauri::command]
async fn interdiff_since_reviewed(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
    to_commit: String,
    ignore_whitespace: bool,
) -> Result<Interdiff, String> {
    let repo = repo_handle(&state, &window)?;
    let repo_key = repo_key(&state, &window)?;
    let from = state
        .review
        .lock()
        .expect("review lock")
        .reviewed_commit(&repo_key, &change_id)
        .ok_or_else(|| "change has no reviewed commit recorded".to_string())?;
    blocking(move || {
        let patch = vcs(repo.interdiff(&from, &to_commit, ignore_whitespace))?;
        Ok(Interdiff {
            files: jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())?,
            from_commit: from,
            to_commit,
        })
    })
    .await
}

/// Every visible commit under `change_id` — the sides of a divergent change.
///
/// One element for an ordinary change, so the frontend has no second code path for the
/// common case. Not derivable from the graph already loaded: the default revset is
/// `ancestors(@ | bookmarks())` and a divergent sibling is usually neither, so the pane
/// can know a change is divergent (the flag is per commit) and still have nothing to
/// offer as the other version.
#[tauri::command]
async fn change_commits(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<Vec<Change>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.commits_of_change(&change_id))).await
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

/// Tell every window showing the repository identified by `key` that it moved. Scoped by
/// repo rather than by window: a mutation is visible to all windows showing that repo, and
/// to none of the others.
///
/// `key` is a [`Repo::review_key`] — the *repository*, not the workspace. The op log is
/// repo-wide, so a commit made in one workspace changes what every other one is looking at,
/// and a window showing a second workspace would otherwise sit on a stale graph until
/// something happened to touch its own tree. Matching on the workspace root was the same
/// thing right up until there was more than one.
fn emit_repo_changed(app: &AppHandle, state: &AppState, key: &Path) {
    let labels: Vec<String> = state
        .windows
        .lock()
        .expect("windows lock")
        .iter()
        .filter(|(_, window)| window.repo.as_ref().is_some_and(|repo| repo.review_key() == key))
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
    let key = repo.review_key();
    let outcome = blocking(move || vcs(task(&repo))).await?;
    emit_repo_changed(app, state, &key);
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

/// The jjdiff binary, and the argv that makes it apply `plan` as jj's diff editor.
///
/// The plan travels to a child process rather than being applied in-process,
/// because the only seam jj offers for a partial selection is its diff editor —
/// see [`split`] and [`Repo::split_with_diff_editor`]. The temp file comes back
/// to the caller so it can be moved into the blocking closure: it has to outlive
/// the `jj` call and be deleted however that call ends.
fn diff_editor_invocation(
    plan: &split::SplitPlan,
) -> Result<(std::path::PathBuf, tempfile::NamedTempFile, Vec<String>), String> {
    let program = std::env::current_exe()
        .map_err(|error| format!("cannot resolve the jjdiff binary to run as a diff editor: {error}"))?;

    let mut file = tempfile::Builder::new()
        .prefix("jjdiff-plan-")
        .suffix(".json")
        .tempfile()
        .map_err(|error| format!("cannot write the plan: {error}"))?;
    serde_json::to_writer(&mut file, plan)
        .map_err(|error| format!("cannot write the plan: {error}"))?;
    use std::io::Write;
    file.flush().map_err(|error| format!("cannot write the plan: {error}"))?;

    let edit_args = vec![
        "--apply-split-plan".to_string(),
        file.path().to_string_lossy().into_owned(),
        // jj substitutes these with the two directories it checked out.
        "$left".to_string(),
        "$right".to_string(),
    ];
    Ok((program, file, edit_args))
}

/// Hunk-level split: the selected hunks become their own change, the rest stay.
#[tauri::command]
async fn split_hunks(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
    plan: split::SplitPlan,
    message: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    plan.divides()?;
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    let (program, file, edit_args) = diff_editor_invocation(&plan)?;
    run_mutation(&app, &state, repo, move |repo| {
        let outcome = repo.split_with_diff_editor(&revset, &program, &edit_args, &message);
        drop(file);
        outcome
    })
    .await
}

/// Hunk-level squash: the selected hunks move into `into`, the rest stay in `from`.
///
/// The same plan and the same child process as [`split_hunks`] — `jj squash -i`
/// lays out the source's own diff for its editor, which is the diff the plan was
/// built from. What differs is the validation: a squash may legitimately take
/// every hunk, where a split may not (see [`split::SplitPlan::moves`]).
#[tauri::command]
async fn squash_hunks(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    from: String,
    into: String,
    plan: split::SplitPlan,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    plan.moves()?;
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    let (program, file, edit_args) = diff_editor_invocation(&plan)?;
    run_mutation(&app, &state, repo, move |repo| {
        let outcome = repo.squash_with_diff_editor(&from, &into, &program, &edit_args);
        drop(file);
        outcome
    })
    .await
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

/// Point a bookmark at a revision, creating it if it is a new name.
///
/// `allow_backwards` is threaded from the caller rather than decided here, for
/// the reason `allow_immutable` is: it waives one of jj's accident guards, and
/// the frontend is where the confirmation naming what it waives lives.
#[tauri::command]
async fn set_bookmark(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    name: String,
    revset: String,
    allow_backwards: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, move |repo| {
        repo.bookmark_set(&name, &revset, allow_backwards)
    })
    .await
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
) -> Result<Vec<ConflictedFile>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || vcs(repo.conflicts(&revset))).await
}

/// One conflicted file taken apart into its agreed text and its regions.
///
/// Read through `jj file show` rather than off disk even for the working copy:
/// it is the one source that materializes the conflict the same way for every
/// revision, and the on-disk file is whatever the user last saved over it.
#[tauri::command]
async fn conflict_content(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
    path: String,
) -> Result<jjdiff_diff::ConflictedContent, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || {
        let content = vcs(repo.file_content(&revset, &path))?;
        Ok(jjdiff_diff::parse_conflicts(&content))
    })
    .await
}

/// Write a resolution for one conflicted path.
///
/// jjdiff plays jj's merge tool here for the same reason it plays its diff
/// editor for a hunk-level split: `jj resolve` has no non-interactive form, but
/// it does have a protocol. The resolved text is decided in the UI and this
/// hands it over — see [`resolve::apply_resolution`], which refuses text that
/// still holds fences.
#[tauri::command]
async fn resolve_conflict(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: String,
    path: String,
    content: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    if jjdiff_diff::has_conflict_markers(&content) {
        return Err("this resolution still contains conflict markers — every region has to be settled before it can be written".into());
    }
    let repo = repo_handle(&state, &window)?.allowing_immutable(allow_immutable);
    let program = std::env::current_exe()
        .map_err(|error| format!("cannot resolve the jjdiff binary to run as a merge tool: {error}"))?;

    let mut file = tempfile::Builder::new()
        .prefix("jjdiff-resolution-")
        .tempfile()
        .map_err(|error| format!("cannot write the resolution: {error}"))?;
    use std::io::Write;
    file.write_all(content.as_bytes())
        .and_then(|()| file.flush())
        .map_err(|error| format!("cannot write the resolution: {error}"))?;

    let merge_args = vec![
        "--apply-resolution".to_string(),
        file.path().to_string_lossy().into_owned(),
        // jj substitutes this with the file it will read the resolution from.
        "$output".to_string(),
    ];
    run_mutation(&app, &state, repo, move |repo| {
        let outcome = repo.resolve_with_merge_tool(&revset, &path, &program, &merge_args);
        drop(file);
        outcome
    })
    .await
}

#[tauri::command]
fn review_status(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<ReviewStatus, String> {
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
    let imported = blocking(move || {
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {path}: {error}"))?;
        let files = compute_diff(&repo, revset.as_deref(), ignore_whitespace)?;
        // An agent-authored walkthrough was written against the diff itself,
        // whatever the author chose to read of it — not jjdiff's outline.
        walkthrough::parse_response(&raw, &files, false)
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
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
    let cfg = config::load().walkthrough;
    let generated = blocking(move || {
        let files = compute_diff(&repo, revset.as_deref(), ignore_whitespace)?;
        let backend = cfg.cli_backend();
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

/// Write a commit message for `revset` (or the working copy) with the
/// configured agent CLI, and return it for the user to edit.
///
/// Returns the text rather than describing the change itself: a generated
/// message is a draft, and one that committed on arrival would make the button
/// a mutation you cannot preview. `App.saveDescription` is still what writes it.
///
/// The last few descriptions go in the prompt so the message matches the
/// repository's own conventions — see [`describe`]. They come from `@-` and its
/// ancestors, never `@`: the working copy's own description is the thing being
/// replaced, and offering it as an example of house style would have the agent
/// imitate the placeholder it was asked to improve.
#[tauri::command]
async fn generate_description(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    let cfg = config::load();
    blocking(move || {
        let files = compute_diff(&repo, revset.as_deref(), ignore_whitespace)?;
        let recent: Vec<String> = repo
            // `ancestors(x, n)` includes `x`, so six back covers five usable
            // examples even when one of them is an empty merge commit.
            .log("ancestors(@-,6)")
            .map_err(|error| error.to_string())
            .unwrap_or_default()
            .into_iter()
            .filter(|change| !change.description.trim().is_empty())
            .take(5)
            .map(|change| change.description)
            .collect();
        let backend = cfg.walkthrough.cli_backend_for_describe(&cfg.describe);
        describe::generate(&backend, &files, &recent, &cfg.describe.prompt)
    })
    .await
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

/// Where a new workspace called `name` would be created, and whether jjdiff can.
///
/// Asked before creating one so the UI can show the path in the confirmation rather than
/// report it afterwards — a second checkout of a repo is a large thing to appear somewhere
/// unannounced.
#[tauri::command]
async fn workspace_path(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    let root = workspace_root()?;
    Ok(workspaces::generated_path(&root, &repo.review_key(), &name)
        .to_string_lossy()
        .into_owned())
}

/// The configured parent for generated workspaces.
fn workspace_root() -> Result<PathBuf, String> {
    config::load().workspace.resolved_root().ok_or_else(|| {
        "no directory is configured for workspaces — set `[workspace] root` in ~/.config/jjdiff/config.toml".to_string()
    })
}

/// A workspace name nobody has to invent: derived from the change, unique in this repo.
#[tauri::command]
async fn suggest_workspace_name(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    description: String,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || {
        let taken: Vec<String> =
            repo.workspaces().unwrap_or_default().into_iter().map(|w| w.name).collect();
        Ok(workspaces::suggest_name(&description, &taken))
    })
    .await
}

/// Create a workspace. `revisions` are jj's `-r` — parents for its new working copy, *not* a
/// checkout; see [`Repo::workspace_add`].
#[tauri::command]
async fn create_workspace(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    name: String,
    revisions: Vec<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    let path = workspaces::generated_path(&workspace_root()?, &repo.review_key(), &name);
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    run_mutation(&app, &state, repo, move |repo| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(jjdiff_vcs::VcsError::Io)?;
        }
        repo.workspace_add(&path, &name, &revisions)
    })
    .await
}

/// Stop tracking a workspace, and — only if jjdiff created it — remove its directory.
///
/// The two halves are separate because jj keeps them separate: `jj workspace forget` never
/// touches the disk. `delete_files` is therefore a decision the UI has to have put to
/// someone, and it is refused outright for a tree jjdiff did not create, wherever the caller
/// claims it is. The forget is undoable and the deletion is not, which is why the deletion
/// happens second: an undo after a failed forget would otherwise restore a record pointing
/// at files that are gone.
#[tauri::command]
async fn forget_workspace(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    name: String,
    delete_files: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    if repo.workspaces().unwrap_or_default().iter().any(|w| w.current && w.name == name) {
        return Err("this window is showing that workspace — open another one first".into());
    }
    let generated_root = config::load().workspace.resolved_root();
    let path = repo
        .workspaces()
        .unwrap_or_default()
        .into_iter()
        .find(|w| w.name == name)
        .and_then(|w| w.path)
        .map(PathBuf::from);
    let remove = delete_files
        && path
            .as_deref()
            .is_some_and(|path| workspaces::is_generated(generated_root.as_deref(), path));
    if delete_files && !remove {
        return Err(
            "jjdiff only deletes workspaces it created; this one is forgotten but left on disk"
                .into(),
        );
    }
    let outcome = run_mutation(&app, &state, repo, move |repo| repo.workspace_forget(&name)).await?;
    if remove {
        if let Some(path) = path {
            std::fs::remove_dir_all(&path)
                .map_err(|error| format!("forgotten, but {} could not be removed: {error}", path.display()))?;
        }
    }
    Ok(outcome)
}

/// `jj workspace update-stale` for the workspace this window is showing.
#[tauri::command]
async fn update_stale_workspace(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    run_mutation(&app, &state, repo, |repo| repo.workspace_update_stale()).await
}

/// Run `jj edit`/`jj new` in *another* workspace.
///
/// No new verb: a workspace is a `Repo`, so this discovers one for its path and calls the
/// same `edit`/`new_change` the current window uses. What it cannot do is act on a workspace
/// whose directory is gone, which is why the path is resolved rather than assumed.
#[tauri::command]
async fn checkout_in_workspace(
    app: AppHandle,
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    workspace: String,
    revset: String,
    mode: String,
    allow_immutable: bool,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state, &window)?;
    let target = repo
        .workspaces()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|w| w.name == workspace)
        .ok_or_else(|| format!("no workspace called {workspace}"))?;
    let path = target
        .path
        .ok_or_else(|| format!("the directory for {workspace} is gone — forget it or restore it"))?;
    let elsewhere = discover(path).await?.allowing_immutable(allow_immutable);
    run_mutation(&app, &state, elsewhere, move |repo| match mode.as_str() {
        "new" => repo.new_change(&[revset]),
        _ => repo.edit(&revset),
    })
    .await
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
    remember_repo(&state, &review_key(&repo));
    let launch = LaunchOptions {
        repo_path: repo.root().to_path_buf(),
        revset: None,
        walkthrough: false,
        walkthrough_file: None,
        pull_request: None,
    };
    let key = repo.review_key();
    bind_window(&app, &state, window.label(), repo, launch);
    emit_repo_changed(&app, &state, &key);
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
    remember_repo(&state, &review_key(&repo));
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
    let title = window_title(&repo);
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
        .min_inner_size(1024.0, 640.0)
        // `dragDropEnabled: false` for `main` in tauri.conf.json, and there is
        // no way to say it there for a window built here.
        //
        // It is what makes HTML5 drag and drop work inside the page. Tauri's
        // handler makes the WebView an OS drag *destination* so a file dropped
        // from Finder can raise an event, and wry only forwards the AppKit drag
        // to WebKit when that handler declines — Tauri's never does. An
        // in-page drag is an AppKit drag too, so it went to Tauri and stopped:
        // `dragstart` fired (the source side is all WebKit), `dragover` and
        // `drop` never did. A drag you can begin and cannot finish, with no
        // error anywhere — rebase-by-drag and moving a bookmark onto another
        // change both looked implemented and were dead.
        //
        // Free to turn off: jjdiff listens for no file drop. It takes a repo
        // from argv and a second instance, not from the desktop.
        .disable_drag_drop_handler();

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

/// The Open Recent list, with generated workspaces filtered out.
///
/// `remember_repo` only records repositories now, but the file on disk outlives that change
/// and anyone who opened a workspace before it has one stored. Filtering on the way out
/// costs a string comparison and means the list corrects itself rather than waiting for the
/// entry to age off the end.
///
/// The test is the same prefix `workspaces::is_generated` uses, so it is one rule and not
/// two: a workspace jjdiff made lives under `[workspace] root`, and one the user made
/// somewhere of their own choosing is a directory they picked and may reasonably reopen.
#[tauri::command]
fn recent_repos(state: tauri::State<'_, AppState>) -> Vec<String> {
    let generated_root = config::load().workspace.resolved_root();
    state
        .recents
        .lock()
        .expect("recents lock")
        .iter()
        .filter(|path| !workspaces::is_generated(generated_root.as_deref(), Path::new(path)))
        .cloned()
        .collect()
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
    Ok(forge::Client { kind, root: repo.root().to_path_buf(), remote: name.clone() })
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

/// The proposal for one branch, asked for by name rather than found in the list.
/// See [`forge::Client::find_by_head`] — this is what makes the banner appear on
/// a repo with more open proposals than a page holds, and on a merged one.
#[tauri::command]
async fn pull_request_for_branch(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    branch: String,
) -> Result<Option<forge::Summary>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.find_by_head(&branch)).await
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
    let key = repo.review_key();
    let opened = blocking(move || {
        let client = forge_client(&repo)?;
        let pull_request = client.pull_request(number)?;
        let bookmark = vcs(repo.fetch_forge_ref(
            &client.remote,
            &client.kind.head_ref(number),
            &client.kind.local_bookmark(number),
        ))?;
        // The merge-base commit is an ancestor of the base branch, so the base
        // branch has to be local for the review revset to resolve. Not fatal:
        // an already-current repo makes this a no-op, and an offline one still
        // gets a review if it happens to have the commit.
        if let Err(error) = repo.git_fetch_branch(&client.remote, &pull_request.base) {
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
    emit_repo_changed(&app, &state, &key);
    Ok(opened)
}

/// The branch a new proposal should target by default.
///
/// Its own command rather than a field on [`forge_info`]: that runs on every
/// repo open and this is a network call only the compose dialog needs.
#[tauri::command]
async fn default_branch(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.default_branch()).await
}

/// The repository's proposal template, when it has one.
///
/// Beside [`default_branch`] and asked at the same moment, for the same reason:
/// both are things the compose dialog needs and nothing else does. A repo with
/// no forge has no template convention to read, so the missing client is `None`
/// rather than an error — the dialog opens regardless.
#[tauri::command]
async fn proposal_template(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || Ok(forge_client(&repo).ok().and_then(|client| client.template()))).await
}

/// Open a proposal. Outward-facing and public the moment it succeeds — the UI
/// collects and shows every field before this is reached.
///
/// The head branch must already be on the remote: `gh` asks the forge to
/// resolve it, so a bookmark that exists only locally fails here rather than
/// pushing implicitly. The frontend pushes first, through the ordinary
/// [`git_push`] path, so that half is a jj mutation with jj's own narration and
/// an operation to undo — which is exactly what opening a proposal is not.
#[tauri::command]
async fn create_pull_request(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
    request: forge::NewPullRequest,
) -> Result<forge::Created, String> {
    let repo = repo_handle(&state, &window)?;
    blocking(move || forge_client(&repo)?.create(&request)).await
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
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
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
    let repo_key = repo_key(&state, &window)?;
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
                    remember_repo(&state, &review_key(&repo));
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
            set_config_value,
            config_file_path,
            set_menu,
            repo_state,
            diff,
            interdiff_since_reviewed,
            interdiff,
            change_versions,
            change_commits,
            describe,
            new_change,
            squash_paths,
            absorb,
            conflicts,
            conflict_content,
            resolve_conflict,
            workspace_path,
            suggest_workspace_name,
            create_workspace,
            forget_workspace,
            update_stale_workspace,
            checkout_in_workspace,
            review_status,
            set_viewed,
            mark_reviewed,
            get_walkthrough,
            generate_walkthrough,
            generate_description,
            open_repository,
            open_repo_window,
            pick_repository,
            recent_repos,
            install_terminal_helper,
            open_in_editor,
            open_url,
            forge_info,
            list_pull_requests,
            pull_request_for_branch,
            pull_request,
            pull_request_activity,
            open_pull_request,
            default_branch,
            proposal_template,
            create_pull_request,
            submit_review,
            file_content,
            file_bytes,
            import_walkthrough,
            edit_change,
            split_paths,
            split_hunks,
            squash_hunks,
            abandon_change,
            duplicate_change,
            backout_change,
            rebase_change,
            restore_paths,
            set_bookmark,
            delete_bookmark,
            git_fetch,
            git_push,
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
                remember_repo(&state, &review_key(&repo));
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
