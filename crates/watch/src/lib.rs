//! Repository change detection.
//!
//! jjdiff never polls jj. It watches `.jj/repo/op_heads/heads/` — every jj operation (including
//! working-copy snapshots made by the user's own `jj` invocations) moves an op head, so this
//! single small directory is a complete "something changed" signal. Filesystem watching of the
//! working copy itself arrives in M1 alongside fs-vs-tree diffing.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("op heads directory does not exist: {0}")]
    MissingOpHeads(PathBuf),
    #[error(transparent)]
    Notify(#[from] notify::Error),
}

/// Handle keeping the watcher alive; drop to stop watching.
pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
    _thread: std::thread::JoinHandle<()>,
}

/// Invoke `on_change` (debounced) whenever a jj operation lands in the repo at `op_heads_dir`.
///
/// Debouncing collapses the burst of fs events a single operation produces; `debounce` is also
/// the floor between two callbacks, so a `jj` command storm (e.g. a rebase writing many ops)
/// coalesces into few refreshes.
pub fn watch_op_heads(
    op_heads_dir: &Path,
    debounce: Duration,
    on_change: impl Fn() + Send + 'static,
) -> Result<RepoWatcher, WatchError> {
    if !op_heads_dir.is_dir() {
        return Err(WatchError::MissingOpHeads(op_heads_dir.to_path_buf()));
    }

    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        if result.is_ok() {
            let _ = tx.send(());
        }
    })?;
    watcher.watch(op_heads_dir, RecursiveMode::NonRecursive)?;

    let thread = std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Drain the burst, then fire once.
            while rx.recv_timeout(debounce).is_ok() {}
            on_change();
        }
    });

    Ok(RepoWatcher { _watcher: watcher, _thread: thread })
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
            Err(WatchError::MissingOpHeads(_))
        ));
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

        // A burst of writes, like one jj operation replacing the op head.
        for index in 0..5 {
            std::fs::write(tmp.path().join(format!("head-{index}")), "x").unwrap();
        }

        // fsevents latency on macOS can be substantial; poll generously.
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
}
