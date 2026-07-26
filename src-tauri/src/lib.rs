//! jjdiff application shell: launch options, app state, IPC commands, repo watchers.
//!
//! Commands that call jj or walk the filesystem are `async` and run their blocking work on
//! the runtime's blocking pool — sync Tauri commands execute on the main thread, and a slow
//! `jj` invocation there would freeze the window.

mod config;
mod viewed;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{Change, Repo};
use jjdiff_watch::RepoWatcher;
use viewed::ReviewStore;

/// `jjdiff [revset] [-R|--repo <path>]`
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchOptions {
    pub repo_path: PathBuf,
    pub revset: Option<String>,
}

impl LaunchOptions {
    fn from_env() -> LaunchOptions {
        let mut repo_path: Option<PathBuf> = None;
        let mut revset: Option<String> = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-R" | "--repo" => repo_path = args.next().map(PathBuf::from),
                // Ignore unknown flags (tauri dev passes its own).
                flag if flag.starts_with('-') => {}
                positional if revset.is_none() => revset = Some(positional.to_string()),
                _ => {}
            }
        }
        let repo_path = repo_path
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        LaunchOptions { repo_path, revset }
    }
}

struct AppState {
    launch: LaunchOptions,
    repo: Mutex<Option<Repo>>,
    review: Mutex<ReviewStore>,
    _watchers: Mutex<Vec<RepoWatcher>>,
}

/// Serializable snapshot for the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RepoState {
    root: PathBuf,
    jj_version: String,
    working_copy: Change,
    stack: Vec<Change>,
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
        *guard = Some(Repo::discover(&state.launch.repo_path).map_err(|e| e.to_string())?);
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
async fn repo_state(state: tauri::State<'_, AppState>) -> Result<RepoState, String> {
    let repo = repo_handle(&state)?;
    blocking(move || {
        Ok(RepoState {
            root: repo.root().to_path_buf(),
            jj_version: vcs(repo.jj_version())?,
            working_copy: vcs(repo.working_copy())?,
            stack: vcs(repo.stack())?,
        })
    })
    .await
}

/// Structured diff for one revision — or the live working copy when `revset` is `None`.
///
/// Working-copy diffs never touch jj: they run fs-vs-`@-` through gix (no snapshot, no
/// operation written). Revset diffs parse `jj diff --git` output.
#[tauri::command]
async fn diff(
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<Vec<FilePatch>, String> {
    let repo = repo_handle(&state)?;
    blocking(move || match revset {
        Some(revset) => {
            let patch = vcs(repo.patch_for(&revset, ignore_whitespace))?;
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
    })
    .await
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

#[tauri::command]
async fn describe(
    state: tauri::State<'_, AppState>,
    change_id: String,
    message: String,
) -> Result<(), String> {
    let repo = repo_handle(&state)?;
    blocking(move || vcs(repo.describe(&change_id, &message))).await
}

#[tauri::command]
async fn new_change(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let repo = repo_handle(&state)?;
    blocking(move || vcs(repo.new_change())).await
}

/// Move working-copy `paths` into `into` (defaults to the parent): jj-native partial commit.
#[tauri::command]
async fn squash_paths(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
    into: Option<String>,
) -> Result<(), String> {
    let repo = repo_handle(&state)?;
    blocking(move || {
        let target = into.as_deref().unwrap_or("@-");
        vcs(repo.squash_paths("@", target, &paths))
    })
    .await
}

/// `jj absorb`: returns jj's summary of which hunks moved into which changes.
#[tauri::command]
async fn absorb(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let repo = repo_handle(&state)?;
    blocking(move || vcs(repo.absorb())).await
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

pub fn run() {
    let launch = LaunchOptions::from_env();
    let state = AppState {
        launch,
        repo: Mutex::new(None),
        review: Mutex::new(ReviewStore::default()),
        _watchers: Mutex::new(Vec::new()),
    };

    tauri::Builder::default()
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
            mark_reviewed
        ])
        .setup(|app| {
            let state = app.state::<AppState>();

            let review_path = app
                .path()
                .app_data_dir()
                .map(|dir| dir.join("review.json"))
                .unwrap_or_else(|_| PathBuf::from(".jjdiff-review.json"));
            *state.review.lock().expect("review lock") = ReviewStore::load(review_path);

            // Watchers are not fatal: without them the UI still works, it just won't
            // live-refresh.
            if let Ok(repo) = Repo::discover(&state.launch.repo_path) {
                if let Some(window) = app.get_webview_window("main") {
                    let name = repo
                        .root()
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let _ = window.set_title(&format!("jjdiff — {name}"));
                }
                let mut watchers = Vec::new();

                let handle = app.handle().clone();
                match jjdiff_watch::watch_op_heads(
                    &repo.op_heads_dir(),
                    Duration::from_millis(250),
                    move || {
                        let _ = handle.emit("repo-changed", ());
                    },
                ) {
                    Ok(watcher) => watchers.push(watcher),
                    Err(error) => eprintln!("jjdiff: op watcher disabled: {error}"),
                }

                let handle = app.handle().clone();
                match jjdiff_watch::watch_working_copy(
                    repo.root(),
                    Duration::from_millis(400),
                    move || {
                        let _ = handle.emit("repo-changed", ());
                    },
                ) {
                    Ok(watcher) => watchers.push(watcher),
                    Err(error) => eprintln!("jjdiff: fs watcher disabled: {error}"),
                }

                *state._watchers.lock().expect("watcher lock") = watchers;
                *state.repo.lock().expect("repo lock") = Some(repo);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running jjdiff");
}
