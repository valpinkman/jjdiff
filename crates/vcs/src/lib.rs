//! jj facade for jjdiff.
//!
//! Discipline (see PLAN.md):
//! - Every *read* runs with `--ignore-working-copy --color=never --no-pager` so it never
//!   snapshots, never contends for the working-copy lock, and never writes to the op log.
//! - Every *mutation* goes through the real CLI without `--ignore-working-copy` so snapshot
//!   semantics match what the user's own `jj` does.
//! - Structured output comes from `-T` templates built on `json(...)` (JSONL contract),
//!   never from parsing human-formatted output.

mod change;
mod runner;

pub use change::{Change, Signature};
pub use runner::JjRunner;

use std::path::{Path, PathBuf};

pub const MIN_JJ_VERSION: (u32, u32) = (0, 33); // json() template support

#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error("`{bin}` was not found on PATH (set JJDIFF_JJ_PATH to override)")]
    JjNotFound { bin: String },
    #[error("no jj repository found at or above {0}")]
    NotARepo(PathBuf),
    #[error("{0} is a jj repository, but it is not colocated (no .git); run `jj git init --colocate`")]
    NotColocated(PathBuf),
    #[error("jj {args:?} failed: {stderr}")]
    CommandFailed { args: Vec<String>, stderr: String },
    #[error("failed to parse jj output: {0}")]
    Parse(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, VcsError>;

/// A discovered, colocated jj repository.
pub struct Repo {
    root: PathBuf,
    runner: JjRunner,
}

/// Template producing one JSON object per revision (JSONL). Field names match
/// [`change::LogRecord`]. `\"` escapes are jj template-language escapes, not Rust's.
const LOG_TEMPLATE: &str = r#""{\"commit\":" ++ json(self) ++ ",\"empty\":" ++ json(empty) ++ ",\"conflict\":" ++ json(conflict) ++ ",\"immutable\":" ++ json(immutable) ++ ",\"working_copy\":" ++ json(current_working_copy) ++ ",\"bookmarks\":" ++ json(bookmarks.map(|b| b.name())) ++ "}\n""#;

impl Repo {
    /// Find the jj workspace containing `path` and verify it is colocated.
    pub fn discover(path: &Path) -> Result<Repo> {
        let runner = JjRunner::new(path.to_path_buf());
        let root = match runner.read(&["root"]) {
            Ok(out) => PathBuf::from(out.trim_end()),
            Err(VcsError::CommandFailed { stderr, .. }) if stderr.contains("no jj repo") => {
                return Err(VcsError::NotARepo(path.to_path_buf()));
            }
            Err(other) => return Err(other),
        };
        if !root.join(".git").exists() {
            return Err(VcsError::NotColocated(root));
        }
        Ok(Repo { runner: JjRunner::new(root.clone()), root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `jj --version` → e.g. "0.43.0".
    pub fn jj_version(&self) -> Result<String> {
        let out = self.runner.read(&["--version"])?;
        Ok(out.trim().trim_start_matches("jj ").to_string())
    }

    /// Changes matching `revset`, newest-first (jj log order).
    pub fn log(&self, revset: &str) -> Result<Vec<Change>> {
        let out = self.runner.read(&[
            "log", "--no-graph", "-r", revset, "-T", LOG_TEMPLATE,
        ])?;
        out.lines()
            .filter(|l| !l.is_empty())
            .map(change::parse_record)
            .collect()
    }

    /// The current stack: everything mutable on the way to `@`.
    pub fn stack(&self) -> Result<Vec<Change>> {
        self.log("trunk()..@ | @")
    }

    pub fn working_copy(&self) -> Result<Change> {
        self.log("@")?
            .into_iter()
            .next()
            .ok_or_else(|| VcsError::Parse("empty log for @".into()))
    }

    /// Git-format patch for a single revision (its diff against parents).
    pub fn patch_for(&self, revset: &str, ignore_whitespace: bool) -> Result<String> {
        let mut args = vec!["diff", "--git", "--context", "3", "-r", revset];
        if ignore_whitespace {
            args.push("--ignore-all-space");
        }
        self.runner.read(&args)
    }

    /// Git-format patch between two revsets.
    pub fn patch_between(&self, from: &str, to: &str, ignore_whitespace: bool) -> Result<String> {
        let mut args = vec!["diff", "--git", "--context", "3", "--from", from, "--to", to];
        if ignore_whitespace {
            args.push("--ignore-all-space");
        }
        self.runner.read(&args)
    }

    /// Git commit id of the working copy's first parent — the base for live fs-vs-tree
    /// diffing (`jjdiff-diff::worktree`). `None` when `@` sits directly on the root commit,
    /// whose tree is empty (the all-zeros id is not a real git object).
    pub fn working_copy_parent(&self) -> Result<Option<String>> {
        let wc = self.working_copy()?;
        Ok(wc
            .parents
            .into_iter()
            .next()
            .filter(|id| !id.bytes().all(|b| b == b'0')))
    }

    // -- Mutations (jj-native verbs; no staging axis) --

    pub fn describe(&self, change_id: &str, message: &str) -> Result<()> {
        self.runner
            .mutate(&["describe", "-r", change_id, "-m", message])?;
        Ok(())
    }

    pub fn new_change(&self) -> Result<()> {
        self.runner.mutate(&["new"])?;
        Ok(())
    }

    /// Move `paths` (all paths when empty) from one change into another.
    pub fn squash_paths(&self, from: &str, into: &str, paths: &[String]) -> Result<()> {
        let mut args: Vec<&str> = vec!["squash", "--from", from, "--into", into];
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        self.runner.mutate(&args)?;
        Ok(())
    }

    pub fn user_identity(&self) -> Result<(String, String)> {
        let name = self.runner.read(&["config", "get", "user.name"])?;
        let email = self.runner.read(&["config", "get", "user.email"])?;
        Ok((name.trim().to_string(), email.trim().to_string()))
    }

    /// Directory whose contents change whenever an operation lands (watch target).
    pub fn op_heads_dir(&self) -> PathBuf {
        self.root.join(".jj").join("repo").join("op_heads").join("heads")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn jj_available() -> bool {
        Command::new("jj").arg("--version").output().is_ok()
    }

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let out = Command::new("jj")
                .args(args)
                .current_dir(dir)
                .env("JJ_USER", "Test")
                .env("JJ_EMAIL", "test@example.com")
                .output()
                .expect("jj runs");
            assert!(out.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["git", "init", "--colocate", "."]);
        std::fs::write(dir.join("hello.txt"), "hello\n").unwrap();
        run(&["--config", "user.name=Test", "--config", "user.email=test@example.com", "commit", "-m", "initial"]);
    }

    #[test]
    fn discover_rejects_plain_directories() {
        let tmp = tempfile::tempdir().unwrap();
        match Repo::discover(tmp.path()) {
            Err(VcsError::NotARepo(_)) => {}
            other => panic!("expected NotARepo, got {other:?}", other = other.err()),
        }
    }

    #[test]
    fn log_and_stack_roundtrip() {
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();
        assert!(repo.jj_version().unwrap().starts_with('0'));

        let wc = repo.working_copy().unwrap();
        assert!(wc.working_copy);
        assert_eq!(wc.change_id.len(), 32);

        let all = repo.log("all()").unwrap();
        assert!(all.len() >= 2, "root + initial + wc");
        let initial = all.iter().find(|c| c.description.starts_with("initial"));
        assert!(initial.is_some());

        // Read calls must not create operations: repeated logs are stable.
        let again = repo.log("all()").unwrap();
        assert_eq!(all.len(), again.len());
    }
}
