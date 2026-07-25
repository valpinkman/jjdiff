//! jjdiff application shell: launch options, app state, IPC commands, repo watcher.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{Change, Repo, VcsError};
use jjdiff_watch::RepoWatcher;

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
    _watcher: Mutex<Option<RepoWatcher>>,
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

fn with_repo<T>(
    state: &tauri::State<'_, AppState>,
    f: impl FnOnce(&Repo) -> Result<T, VcsError>,
) -> Result<T, String> {
    let mut guard = state.repo.lock().expect("repo lock");
    if guard.is_none() {
        *guard = Some(Repo::discover(&state.launch.repo_path).map_err(|e| e.to_string())?);
    }
    f(guard.as_ref().expect("repo present")).map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_options(state: tauri::State<'_, AppState>) -> LaunchOptions {
    state.launch.clone()
}

#[tauri::command]
fn repo_state(state: tauri::State<'_, AppState>) -> Result<RepoState, String> {
    with_repo(&state, |repo| {
        Ok(RepoState {
            root: repo.root().to_path_buf(),
            jj_version: repo.jj_version()?,
            working_copy: repo.working_copy()?,
            stack: repo.stack()?,
        })
    })
}

/// Structured diff for one revision — or the live working copy when `revset` is `None`.
#[tauri::command]
fn diff(state: tauri::State<'_, AppState>, revset: Option<String>) -> Result<Vec<FilePatch>, String> {
    let patch = with_repo(&state, |repo| match &revset {
        Some(revset) => repo.patch_for(revset),
        None => repo.working_copy_patch(),
    })?;
    jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())
}

#[tauri::command]
fn describe(
    state: tauri::State<'_, AppState>,
    change_id: String,
    message: String,
) -> Result<(), String> {
    with_repo(&state, |repo| repo.describe(&change_id, &message))
}

#[tauri::command]
fn new_change(state: tauri::State<'_, AppState>) -> Result<(), String> {
    with_repo(&state, |repo| repo.new_change())
}

pub fn run() {
    let launch = LaunchOptions::from_env();
    let state = AppState { launch, repo: Mutex::new(None), _watcher: Mutex::new(None) };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            launch_options,
            repo_state,
            diff,
            describe,
            new_change
        ])
        .setup(|app| {
            let state = app.state::<AppState>();
            // Start the op-head watcher if we're in a repo; a failure here is not fatal —
            // the UI still works, it just won't live-refresh.
            if let Ok(repo) = Repo::discover(&state.launch.repo_path) {
                let handle = app.handle().clone();
                match jjdiff_watch::watch_op_heads(
                    &repo.op_heads_dir(),
                    Duration::from_millis(250),
                    move || {
                        let _ = handle.emit("repo-changed", ());
                    },
                ) {
                    Ok(watcher) => {
                        *state._watcher.lock().expect("watcher lock") = Some(watcher);
                    }
                    Err(error) => eprintln!("jjdiff: watcher disabled: {error}"),
                }
                *state.repo.lock().expect("repo lock") = Some(repo);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running jjdiff");
}
