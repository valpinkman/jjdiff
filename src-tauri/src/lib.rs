//! jjdiff application shell: launch options, app state, IPC commands, repo watchers.
//!
//! Commands that call jj or walk the filesystem are `async` and run their blocking work on
//! the runtime's blocking pool — sync Tauri commands execute on the main thread, and a slow
//! `jj` invocation there would freeze the window.

mod config;
mod viewed;
pub mod walkthrough;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{Change, Repo};
use jjdiff_watch::RepoWatcher;
use viewed::ReviewStore;

/// `jjdiff [revset] [-R|--repo <path>] [-w|--walkthrough] [--walkthrough-file <path>]`
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
    fn from_env() -> LaunchOptions {
        let mut repo_path: Option<PathBuf> = None;
        let mut revset: Option<String> = None;
        let mut walkthrough = false;
        let mut walkthrough_file: Option<PathBuf> = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-R" | "--repo" => repo_path = args.next().map(PathBuf::from),
                "-w" | "--walkthrough" => walkthrough = true,
                "--walkthrough-file" => walkthrough_file = args.next().map(PathBuf::from),
                // Ignore unknown flags (tauri dev passes its own).
                flag if flag.starts_with('-') => {}
                positional if revset.is_none() => revset = Some(positional.to_string()),
                _ => {}
            }
        }
        let repo_path = repo_path
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        LaunchOptions { repo_path, revset, walkthrough, walkthrough_file }
    }
}

struct AppState {
    launch: LaunchOptions,
    repo: Mutex<Option<Repo>>,
    review: Mutex<ReviewStore>,
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
async fn repo_state(state: tauri::State<'_, AppState>) -> Result<RepoState, String> {
    let repo = repo_handle(&state)?;
    blocking(move || {
        Ok(RepoState {
            root: repo.root().to_path_buf(),
            jj_version: vcs(repo.jj_version())?,
            working_copy: vcs(repo.working_copy())?,
            stack: vcs(repo.stack())?,
            graph: vcs(repo.graph(60))?,
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
            std::fs::read_to_string(&full).map_err(|error| {
                format!("cannot read {}: {error}", full.display())
            })
        }
    })
    .await
}

/// Stored walkthrough for a change, plus whether it still matches the current diff.
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
        recents: Mutex::new(Vec::new()),
        recents_path: Mutex::new(None),
        _watchers: Mutex::new(Vec::new()),
    };

    tauri::Builder::default()
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
            file_content,
            import_walkthrough
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
