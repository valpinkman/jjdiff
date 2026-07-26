//! Repository change detection. Two complementary watchers, both debounced, no polling:
//!
//! - [`watch_op_heads`] fires when a jj *operation* lands (`.jj/repo/op_heads/heads/` moves) —
//!   any `jj` command the user runs, including snapshots.
//! - [`watch_working_copy`] fires when *files* change in the working copy — edits that jj has
//!   not seen yet. Without it, a freshly created file only appears after a manual refresh,
//!   because creating a file writes no operation. Events under `.git`/`.jj` and gitignored
//!   paths are filtered out so builds (`target/`, `node_modules/`) don't storm the UI.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("directory does not exist: {0}")]
    MissingDir(PathBuf),
    #[error(transparent)]
    Notify(#[from] notify::Error),
}

/// Handle keeping a watcher alive; drop to stop watching.
pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
    _thread: std::thread::JoinHandle<()>,
}

/// Invoke `on_change` (debounced) whenever a jj operation lands in the repo.
pub fn watch_op_heads(
    op_heads_dir: &Path,
    debounce: Duration,
    on_change: impl Fn() + Send + 'static,
) -> Result<RepoWatcher, WatchError> {
    if !op_heads_dir.is_dir() {
        return Err(WatchError::MissingDir(op_heads_dir.to_path_buf()));
    }
    start(op_heads_dir, RecursiveMode::NonRecursive, debounce, on_change, |_| true)
}

/// Invoke `on_change` (debounced) whenever a relevant working-copy file changes.
pub fn watch_working_copy(
    repo_root: &Path,
    debounce: Duration,
    on_change: impl Fn() + Send + 'static,
) -> Result<RepoWatcher, WatchError> {
    if !repo_root.is_dir() {
        return Err(WatchError::MissingDir(repo_root.to_path_buf()));
    }
    let filter = PathFilter::new(repo_root);
    start(repo_root, RecursiveMode::Recursive, debounce, on_change, move |event| {
        event.paths.iter().any(|path| filter.is_relevant(path))
    })
}

fn start(
    path: &Path,
    mode: RecursiveMode,
    debounce: Duration,
    on_change: impl Fn() + Send + 'static,
    relevant: impl Fn(&notify::Event) -> bool + Send + 'static,
) -> Result<RepoWatcher, WatchError> {
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if let Ok(event) = result {
            if relevant(&event) {
                let _ = tx.send(());
            }
        }
    })?;
    watcher.watch(path, mode)?;

    let thread = std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Drain the burst, then fire once.
            while rx.recv_timeout(debounce).is_ok() {}
            on_change();
        }
    });

    Ok(RepoWatcher { _watcher: watcher, _thread: thread })
}

/// Collect the repo's `.gitignore` files — root plus nested — as one matcher per directory.
///
/// They cannot be merged into a single [`Gitignore`]: a matcher resolves its patterns
/// against the base directory it was built with, so folding `web/.gitignore` into a
/// root-based matcher would apply `*.map` repo-wide. Each file therefore keeps its own
/// directory as base, and matching walks the applicable ones.
///
/// The walk that finds them is itself gitignore-aware, so an ignored directory's own
/// `.gitignore` is never read (and `target/`-sized trees are never descended). Built when a
/// watcher starts; a newly added `.gitignore` takes effect on the next app run.
fn build_ignores(root: &Path) -> Vec<(PathBuf, Gitignore)> {
    let mut ignores = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| entry.file_name() != ".git" && entry.file_name() != ".jj")
        .build();
    for entry in walker.flatten() {
        if entry.file_name() != ".gitignore" || !entry.path().is_file() {
            continue;
        }
        let Some(dir) = entry.path().parent() else { continue };
        let mut builder = GitignoreBuilder::new(dir);
        // add() returns Some(error) on failure; a broken file must not sink the rest.
        if builder.add(entry.path()).is_some() {
            continue;
        }
        if let Ok(gitignore) = builder.build() {
            ignores.push((dir.to_path_buf(), gitignore));
        }
    }
    // Deepest last: nested rules are evaluated after the ones they refine.
    ignores.sort_by_key(|(dir, _)| dir.components().count());
    ignores
}

/// Decides which fs-event paths matter: not under `.git`/`.jj`, not gitignored.
struct PathFilter {
    root: PathBuf,
    gitignores: Vec<(PathBuf, Gitignore)>,
}

impl PathFilter {
    fn new(root: &Path) -> PathFilter {
        // fsevents on macOS reports symlink-resolved paths (/tmp → /private/tmp, /var →
        // /private/var); the root must be canonical or strip_prefix rejects every event.
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let gitignores = build_ignores(&root);
        PathFilter { root, gitignores }
    }

    fn is_relevant(&self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.root) else {
            return false;
        };
        for component in relative.components() {
            let name = component.as_os_str();
            if name == ".git" || name == ".jj" {
                return false;
            }
        }
        let is_dir = path.is_dir();
        for (dir, gitignore) in &self.gitignores {
            // A .gitignore only governs its own subtree.
            let Ok(scoped) = path.strip_prefix(dir) else { continue };
            if gitignore.matched_path_or_any_parents(scoped, is_dir).is_ignore() {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn missing_dir_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        assert!(matches!(
            watch_op_heads(&missing, Duration::from_millis(10), || {}),
            Err(WatchError::MissingDir(_))
        ));
        assert!(matches!(
            watch_working_copy(&missing, Duration::from_millis(10), || {}),
            Err(WatchError::MissingDir(_))
        ));
    }

    #[test]
    fn path_filter_honours_nested_gitignores() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let root = root.as_path();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("web/dist")).unwrap();
        std::fs::write(root.join("web/.gitignore"), "dist/\n*.map\n").unwrap();
        let filter = PathFilter::new(root);

        assert!(filter.is_relevant(&root.join("web/src/app.ts")));
        // Nested rules apply relative to their own directory...
        assert!(!filter.is_relevant(&root.join("web/dist/bundle.js")));
        assert!(!filter.is_relevant(&root.join("web/app.js.map")));
        // ...and do not leak outside it.
        assert!(filter.is_relevant(&root.join("other/app.js.map")));
        // Root rules still hold.
        assert!(!filter.is_relevant(&root.join("target/debug/x")));
    }

    #[test]
    fn path_filter_skips_vcs_dirs_and_gitignored() {
        let tmp = tempfile::tempdir().unwrap();
        // Canonicalize like fsevents does, so joined paths match the filter's root.
        let root = tmp.path().canonicalize().unwrap();
        let root = root.as_path();
        std::fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir(root.join("target")).unwrap();
        let filter = PathFilter::new(root);

        assert!(filter.is_relevant(&root.join("src.rs")));
        assert!(filter.is_relevant(&root.join("deep/nested/file.ts")));
        assert!(filter.is_relevant(&root.join(".gitignore")));
        assert!(!filter.is_relevant(&root.join(".git/index")));
        assert!(!filter.is_relevant(&root.join(".jj/repo/op_heads/heads/x")));
        assert!(!filter.is_relevant(&root.join("target/debug/build.rs")));
        assert!(!filter.is_relevant(&root.join("debug.log")));
        assert!(!filter.is_relevant(Path::new("/outside/the/repo.txt")));
    }

    #[test]
    fn fires_once_per_burst() {
        let tmp = tempfile::tempdir().unwrap();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = fired.clone();
        let _watcher =
            watch_op_heads(tmp.path(), Duration::from_millis(150), move || {
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();

        for index in 0..5 {
            std::fs::write(tmp.path().join(format!("head-{index}")), "x").unwrap();
        }

        for _ in 0..40 {
            if fired.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let count = fired.load(Ordering::SeqCst);
        assert!(count >= 1, "watcher never fired");
        assert!(count <= 2, "burst was not debounced: {count} callbacks");
    }

    #[test]
    fn working_copy_watcher_ignores_filtered_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".gitignore"), "junk/\n").unwrap();
        std::fs::create_dir(root.join("junk")).unwrap();
        std::fs::create_dir(root.join(".jj")).unwrap();

        let fired = Arc::new(AtomicUsize::new(0));
        let counter = fired.clone();
        let _watcher =
            watch_working_copy(root, Duration::from_millis(100), move || {
                counter.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();

        // Filtered writes only — must not fire.
        std::fs::write(root.join("junk/noise.txt"), "x").unwrap();
        std::fs::write(root.join(".jj/lock"), "x").unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(fired.load(Ordering::SeqCst), 0, "filtered paths fired the watcher");

        // A real file must fire.
        std::fs::write(root.join("real.txt"), "hello").unwrap();
        for _ in 0..40 {
            if fired.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(fired.load(Ordering::SeqCst) >= 1, "real file change did not fire");
    }
}
