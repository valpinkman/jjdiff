//! jjdiff application shell: launch options, app state, IPC commands, repo watchers.

mod config;
mod viewed;

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, Manager};

use jjdiff_diff::FilePatch;
use jjdiff_vcs::{Change, Repo, VcsError};
use jjdiff_watch::RepoWatcher;
use viewed::ViewedStore;

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
    viewed: Mutex<ViewedStore>,
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

fn repo_root_string(state: &tauri::State<'_, AppState>) -> Result<String, String> {
    with_repo(state, |repo| Ok(repo.root().to_string_lossy().into_owned()))
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
///
/// Working-copy diffs never touch jj: they run fs-vs-`@-` through gix (no snapshot, no
/// operation written). Revset diffs parse `jj diff --git` output.
#[tauri::command]
fn diff(
    state: tauri::State<'_, AppState>,
    revset: Option<String>,
    ignore_whitespace: bool,
) -> Result<Vec<FilePatch>, String> {
    match revset {
        Some(revset) => {
            let patch = with_repo(&state, |repo| repo.patch_for(&revset, ignore_whitespace))?;
            jjdiff_diff::parse_git_patch(&patch).map_err(|e| e.to_string())
        }
        None => {
            let (root, base) = with_repo(&state, |repo| {
                Ok((repo.root().to_path_buf(), repo.working_copy_parent()?))
            })?;
            jjdiff_diff::worktree::diff_worktree(
                &root,
                base.as_deref(),
                jjdiff_diff::worktree::WorktreeDiffOptions { ignore_whitespace },
            )
            .map_err(|e| e.to_string())
        }
    }
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

/// Move `paths` of the working copy into its parent (jj-native partial commit).
#[tauri::command]
fn squash_paths(state: tauri::State<'_, AppState>, paths: Vec<String>) -> Result<(), String> {
    with_repo(&state, |repo| repo.squash_paths("@", "@-", &paths))
}

#[tauri::command]
fn viewed_files(
    state: tauri::State<'_, AppState>,
    change_id: String,
) -> Result<Vec<String>, String> {
    let repo = repo_root_string(&state)?;
    Ok(state.viewed.lock().expect("viewed lock").viewed(&repo, &change_id))
}

#[tauri::command]
fn set_viewed(
    state: tauri::State<'_, AppState>,
    change_id: String,
    path: String,
    viewed: bool,
) -> Result<(), String> {
    let repo = repo_root_string(&state)?;
    state.viewed.lock().expect("viewed lock").set(&repo, &change_id, &path, viewed);
    Ok(())
}

pub fn run() {
    let launch = LaunchOptions::from_env();
    let state = AppState {
        launch,
        repo: Mutex::new(None),
        viewed: Mutex::new(ViewedStore::default()),
        _watchers: Mutex::new(Vec::new()),
    };

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            launch_options,
            get_config,
            repo_state,
            diff,
            describe,
            new_change,
            squash_paths,
            viewed_files,
            set_viewed
        ])
        .setup(|app| {
            let state = app.state::<AppState>();

            let viewed_path = app
                .path()
                .app_data_dir()
                .map(|dir| dir.join("viewed.json"))
                .unwrap_or_else(|_| PathBuf::from(".jjdiff-viewed.json"));
            *state.viewed.lock().expect("viewed lock") = ViewedStore::load(viewed_path);

            // Watchers are not fatal: without them the UI still works, it just won't
            // live-refresh.
            if let Ok(repo) = Repo::discover(&state.launch.repo_path) {
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
