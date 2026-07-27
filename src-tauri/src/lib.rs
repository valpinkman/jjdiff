//! jjdiff application shell: launch options, app state, IPC commands, repo watchers.
//!
//! Commands that call jj or walk the filesystem are `async` and run their blocking work on
//! the runtime's blocking pool — sync Tauri commands execute on the main thread, and a slow
//! `jj` invocation there would freeze the window.

pub mod cli;
mod comments;
mod config;
mod viewed;
pub mod walkthrough;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{Change, Operation, Outcome, Repo};
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
        }
    }
}

struct AppState {
    launch: LaunchOptions,
    repo: Mutex<Option<Repo>>,
    review: Mutex<ReviewStore>,
    comments: Mutex<CommentStore>,
    recents: Mutex<Vec<String>>,
    recents_path: Mutex<Option<PathBuf>>,
    _watchers: Mutex<Vec<RepoWatcher>>,
}

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

/// (Re)start both watchers for `repo` and point the window title at it.
fn attach_repo(app: &AppHandle, repo: &Repo) -> Vec<RepoWatcher> {
    if let Some(window) = app.get_webview_window("main") {
        let name = repo
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = window.set_title(&format!("jjdiff — {name}"));
    }
    let mut watchers = Vec::new();
    let handle = app.clone();
    match jjdiff_watch::watch_op_heads(&repo.op_heads_dir(), Duration::from_millis(250), move || {
        let _ = handle.emit("repo-changed", ());
    }) {
        Ok(watcher) => watchers.push(watcher),
        Err(error) => eprintln!("jjdiff: op watcher disabled: {error}"),
    }
    let handle = app.clone();
    match jjdiff_watch::watch_working_copy(repo.root(), Duration::from_millis(400), move || {
        let _ = handle.emit("repo-changed", ());
    }) {
        Ok(watcher) => watchers.push(watcher),
        Err(error) => eprintln!("jjdiff: fs watcher disabled: {error}"),
    }
    watchers
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

/// Clone the (cheap) repo handle out of state, discovering it on first use.
fn repo_handle(state: &tauri::State<'_, AppState>) -> Result<Repo, String> {
    let mut guard = state.repo.lock().expect("repo lock");
    if guard.is_none() {
        let repo = Repo::discover(&state.launch.repo_path).map_err(|e| e.to_string())?;
        // Fail loudly here rather than with a confusing template parse error later.
        repo.check_version().map_err(|e| e.to_string())?;
        *guard = Some(repo);
    }
    Ok(guard.as_ref().expect("repo present").clone())
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

#[tauri::command]
fn launch_options(state: tauri::State<'_, AppState>) -> LaunchOptions {
    state.launch.clone()
}

#[tauri::command]
fn get_config() -> config::Config {
    config::load()
}

#[tauri::command]
async fn repo_state(
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
) -> Result<RepoState, String> {
    let repo = repo_handle(&state)?;
    let graph_revset = revset.unwrap_or_else(|| "ancestors(@ | bookmarks())".to_string());
    blocking(move || {
        Ok(RepoState {
            root: repo.root().to_path_buf(),
            jj_version: vcs(repo.jj_version())?,
            working_copy: vcs(repo.working_copy())?,
            stack: vcs(repo.stack())?,
            graph: vcs(repo.graph(&graph_revset, 200))?,
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
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<Vec<FilePatch>, String> {
    let repo = repo_handle(&state)?;
    blocking(move || compute_diff(&repo, revset.as_deref(), ignore_whitespace)).await
}

/// Interdiff from the last-reviewed commit of `change_id` to its current commit.
#[tauri::command]
async fn interdiff_since_reviewed(
    state: tauri::State<'_, AppState>,
    change_id: String,
    ignore_whitespace: bool,
) -> Result<Interdiff, String> {
    let repo = repo_handle(&state)?;
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

/// Every mutation follows the same shape: run it off the main thread, then emit
/// `repo-changed` so the UI reloads. The returned [`Outcome`] carries jj's own narration
/// plus the operation id, which is what makes a one-click undo possible.
async fn run_mutation<F>(app: &AppHandle, repo: Repo, task: F) -> Result<Outcome, String>
where
    F: FnOnce(&Repo) -> jjdiff_vcs::Result<Outcome> + Send + 'static,
{
    let outcome = blocking(move || vcs(task(&repo))).await?;
    let _ = app.emit("repo-changed", ());
    Ok(outcome)
}

#[tauri::command]
async fn describe(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    change_id: String,
    message: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.describe(&change_id, &message)).await
}

#[tauri::command]
async fn new_change(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    parents: Vec<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.new_change(&parents)).await
}

/// `jj edit` — move the working copy onto an existing change.
#[tauri::command]
async fn edit_change(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.edit(&revset)).await
}

/// Move working-copy `paths` into `into` (defaults to the parent): jj-native partial commit.
#[tauri::command]
async fn squash_paths(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
    into: Option<String>,
    from: Option<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| {
        repo.squash_paths(
            from.as_deref().unwrap_or("@"),
            into.as_deref().unwrap_or("@-"),
            &paths,
        )
    })
    .await
}

#[tauri::command]
async fn absorb(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, |repo| repo.absorb()).await
}

/// File-level split: `paths` stay in the change, the rest move to a new child.
#[tauri::command]
async fn split_paths(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    revset: String,
    paths: Vec<String>,
) -> Result<Outcome, String> {
    if paths.is_empty() {
        return Err("select at least one file to split out".into());
    }
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.split_paths(&revset, &paths)).await
}

#[tauri::command]
async fn abandon_change(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.abandon(&revset)).await
}

#[tauri::command]
async fn duplicate_change(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.duplicate(&revset)).await
}

#[tauri::command]
async fn backout_change(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.backout(&revset)).await
}

/// `mode` is "revision" (just it), "source" (it and descendants) or "branch".
#[tauri::command]
async fn rebase_change(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    mode: String,
    revset: String,
    destination: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.rebase(&mode, &revset, &destination)).await
}

/// Discard working-copy changes to `paths` (all when empty). Undoable via the op log.
#[tauri::command]
async fn restore_paths(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.restore_paths(&paths)).await
}

#[tauri::command]
async fn set_bookmark(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
    revset: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.bookmark_set(&name, &revset)).await
}

#[tauri::command]
async fn delete_bookmark(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.bookmark_delete(&name)).await
}

#[tauri::command]
async fn git_fetch(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    remote: Option<String>,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.git_fetch(remote.as_deref())).await
}

/// Push a bookmark, or `change` to auto-name one from the change id. The forge prints a
/// ready-made pull-request URL on a branch-creating push; [`pull_request_url`] finds it.
#[tauri::command]
async fn git_push(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    remote: Option<String>,
    bookmark: Option<String>,
    change: Option<String>,
) -> Result<PushResult, String> {
    let repo = repo_handle(&state)?;
    let outcome = run_mutation(&app, repo, move |repo| {
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
async fn remotes(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let repo = repo_handle(&state)?;
    blocking(move || vcs(repo.remotes())).await
}

// -- Operation log / undo --

#[tauri::command]
async fn operation_log(
    state: tauri::State<'_, AppState>,
    limit: usize,
) -> Result<Vec<Operation>, String> {
    let repo = repo_handle(&state)?;
    blocking(move || vcs(repo.operations(limit))).await
}

#[tauri::command]
async fn undo(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, |repo| repo.undo()).await
}

#[tauri::command]
async fn restore_operation(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    operation: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.op_restore(&operation)).await
}

#[tauri::command]
async fn revert_operation(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    operation: String,
) -> Result<Outcome, String> {
    let repo = repo_handle(&state)?;
    run_mutation(&app, repo, move |repo| repo.op_revert(&operation)).await
}

#[tauri::command]
async fn conflicts(
    state: tauri::State<'_, AppState>,
    revset: String,
) -> Result<Vec<String>, String> {
    let repo = repo_handle(&state)?;
    blocking(move || vcs(repo.conflicted_paths(&revset))).await
}

#[tauri::command]
fn review_status(
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<ReviewStatus, String> {
    let repo = repo_handle(&state)?;
    let repo_key = repo.root().to_string_lossy().into_owned();
    let review = state.review.lock().expect("review lock");
    Ok(ReviewStatus {
        viewed: review.viewed(&repo_key, &change_id),
        reviewed_commit: review.reviewed_commit(&repo_key, &change_id),
    })
}

#[tauri::command]
fn set_viewed(
    state: tauri::State<'_, AppState>,
    change_id: String,
    path: String,
    viewed: bool,
) -> Result<(), String> {
    let repo = repo_handle(&state)?;
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
    state: tauri::State<'_, AppState>,
    change_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
    path: String,
) -> Result<walkthrough::Walkthrough, String> {
    let repo = repo_handle(&state)?;
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
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    path: String,
) -> Result<String, String> {
    let repo = repo_handle(&state)?;
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

/// Stored walkthrough for a change/// Stored walkthrough for a change, plus whether it still matches the current diff.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WalkthroughStatus {
    walkthrough: Option<walkthrough::Walkthrough>,
    stale: bool,
}

#[tauri::command]
async fn get_walkthrough(
    state: tauri::State<'_, AppState>,
    change_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<WalkthroughStatus, String> {
    let repo = repo_handle(&state)?;
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
    state: tauri::State<'_, AppState>,
    change_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
    context: String,
) -> Result<walkthrough::Walkthrough, String> {
    let repo = repo_handle(&state)?;
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

/// Switch the app to another repository (must be a colocated jj repo).
#[tauri::command]
async fn open_repository(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let repo = tauri::async_runtime::spawn_blocking(move || {
        let repo = Repo::discover(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
        repo.check_version().map_err(|e| e.to_string())?;
        Ok::<_, String>(repo)
    })
    .await
    .map_err(|e| e.to_string())??;

    let watchers = attach_repo(&app, &repo);
    remember_repo(&state, &repo.root().to_string_lossy());
    *state._watchers.lock().expect("watcher lock") = watchers;
    *state.repo.lock().expect("repo lock") = Some(repo);
    let _ = app.emit("repo-changed", ());
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
    state: tauri::State<'_, AppState>,
    change_id: String,
    commit_id: String,
) -> Result<(), String> {
    let repo = repo_handle(&state)?;
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
    let repo = repo_handle(&state)?;
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
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<Vec<Comment>, String> {
    let repo = repo_handle(&state)?;
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
    state: tauri::State<'_, AppState>,
    change_id: String,
    current_commit_id: String,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<usize, String> {
    let repo = repo_handle(&state)?;
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
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<String, String> {
    let repo = repo_handle(&state)?;
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
        repo: Mutex::new(None),
        review: Mutex::new(ReviewStore::default()),
        comments: Mutex::new(CommentStore::in_memory().expect("comment db bootstrap")),
        recents: Mutex::new(Vec::new()),
        recents_path: Mutex::new(None),
        _watchers: Mutex::new(Vec::new()),
    };

    let mut builder = tauri::Builder::default();

    // Single instance: launching `jjdiff` from a second repo opens a new
    // window in the existing process rather than a rival process fighting
    // over the same review store. We parse the second instance's argv with
    // the same [`Args`] parser and forward a `second-instance` event.
    builder = builder.plugin(
        tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // argv[0] is the binary; the rest mirrors `jjdiff [flags] [revset]`.
            let rest: Vec<String> = argv.iter().skip(1).cloned().collect();
            match Args::parse(&rest) {
                Ok(parsed) => {
                    let _ = app.emit("second-instance", parsed);
                }
                Err(error) => {
                    eprintln!("jjdiff: ignoring second instance with bad args: {error}");
                }
            }
        }),
    );

    builder
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            launch_options,
            get_config,
            repo_state,
            diff,
            interdiff_since_reviewed,
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
            pick_repository,
            recent_repos,
            install_terminal_helper,
            file_content,
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

            // Watchers are not fatal: without them the UI still works, it just won't
            // live-refresh.
            if let Ok(repo) = Repo::discover(&state.launch.repo_path) {
                let watchers = attach_repo(app.handle(), &repo);
                remember_repo(&state, &repo.root().to_string_lossy());
                *state._watchers.lock().expect("watcher lock") = watchers;
                *state.repo.lock().expect("repo lock") = Some(repo);
            }
            Ok(())
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
