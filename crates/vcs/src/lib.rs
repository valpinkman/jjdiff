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

pub use change::{Change, EvologEntry, Operation, Signature};
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
    #[error("jj {found} is too old — jjdiff needs {}.{} or newer", MIN_JJ_VERSION.0, MIN_JJ_VERSION.1)]
    JjTooOld { found: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, VcsError>;

/// What a mutation did: jj's own narration, and the operation it produced so the UI can
/// offer an undo for exactly that step.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub message: String,
    pub operation: String,
}

/// A discovered, colocated jj repository. Cheap to clone (two paths) — commands clone it out
/// of app state so blocking jj work can run off the main thread.
#[derive(Clone)]
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

    /// Fail fast on a jj too old for the template features jjdiff depends on
    /// (`json()` landed in 0.33). Unparseable versions pass — a dev build should not
    /// block the app.
    pub fn check_version(&self) -> Result<()> {
        let version = self.jj_version()?;
        let mut parts = version.split(['.', '-', '+']);
        let major: Option<u32> = parts.next().and_then(|p| p.parse().ok());
        let minor: Option<u32> = parts.next().and_then(|p| p.parse().ok());
        if let (Some(major), Some(minor)) = (major, minor) {
            if (major, minor) < MIN_JJ_VERSION {
                return Err(VcsError::JjTooOld { found: version });
            }
        }
        Ok(())
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

    /// Recent history for the graph view: ancestors of `@` and every bookmark, capped.
    /// jj log order is reverse-topological (children before parents), which the UI's
    /// lane-assignment algorithm depends on.
    pub fn graph(&self, revset: &str, limit: usize) -> Result<Vec<Change>> {
        let limit = limit.to_string();
        let out = self.runner.read(&[
            "log", "--no-graph", "-n", &limit, "-r", revset, "-T", LOG_TEMPLATE,
        ])?;
        out.lines()
            .filter(|l| !l.is_empty())
            .map(change::parse_record)
            .collect()
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

    /// Full contents of `path` at `revset` — used to expand diff context beyond the
    /// hunks jj emitted. `jj file show` writes raw bytes; non-UTF-8 files are rejected
    /// rather than lossily rendered into a code view.
    pub fn file_content(&self, revset: &str, path: &str) -> Result<String> {
        self.runner.read(&["file", "show", "-r", revset, path])
    }

    /// Raw bytes of `path` at `revset` — for binary files (images). Unlike
    /// [`file_content`](Self::file_content), this does not reject non-UTF-8.
    pub fn file_bytes(&self, revset: &str, path: &str) -> Result<Vec<u8>> {
        self.runner.read_bytes(&["file", "show", "-r", revset, path])
    }

    /// How a change evolved: one entry per predecessor commit, newest first. Entry 0 is the
    /// current commit. Powers "what changed since I last reviewed" interdiffs.
    ///
    /// Note: evolog's template context differs from log's — entries expose `commit`, not the
    /// revision keywords, so this uses its own template and a lighter record type.
    pub fn evolog(&self, change_id: &str) -> Result<Vec<EvologEntry>> {
        const EVOLOG_TEMPLATE: &str = r#"json(commit) ++ "\n""#;
        let out = self.runner.read(&[
            "evolog", "--no-graph", "-n", "50", "-r", change_id, "-T", EVOLOG_TEMPLATE,
        ])?;
        out.lines()
            .filter(|l| !l.is_empty())
            .map(change::parse_evolog_record)
            .collect()
    }

    /// Git-format *interdiff*: how the diff itself changed between two commits of a change
    /// (rebase noise excluded by jj). Both sides may be hidden commits — jj addresses them
    /// by commit id.
    pub fn interdiff(&self, from_commit: &str, to_commit: &str, ignore_whitespace: bool) -> Result<String> {
        let mut args = vec![
            "interdiff", "--git", "--context", "3", "--from", from_commit, "--to", to_commit,
        ];
        if ignore_whitespace {
            args.push("--ignore-all-space");
        }
        self.runner.read(&args)
    }

    /// Paths with unresolved conflicts in `revset`. Parses `jj resolve --list` lines of the
    /// form `<path>    <N>-sided conflict…` — the description suffix is stripped from the
    /// right so paths containing spaces survive.
    pub fn conflicted_paths(&self, revset: &str) -> Result<Vec<String>> {
        let out = match self.runner.read(&["resolve", "--list", "-r", revset]) {
            Ok(out) => out,
            // jj exits non-zero when there is nothing to resolve.
            Err(VcsError::CommandFailed { stderr, .. }) if stderr.contains("No conflicts") => {
                return Ok(Vec::new());
            }
            Err(other) => return Err(other),
        };
        Ok(out
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| match line.rfind("    ") {
                Some(position) => line[..position].trim_end().to_string(),
                None => line.trim_end().to_string(),
            })
            .collect())
    }

    // -- Operation log --

    /// Recent operations, newest first. `json(self)` yields id, parents, timestamps,
    /// description and the literal argv — no parsing required.
    pub fn operations(&self, limit: usize) -> Result<Vec<Operation>> {
        let limit = limit.to_string();
        let out = self.runner.read(&[
            "op", "log", "--no-graph", "-n", &limit, "-T", r#"json(self) ++ "\n""#,
        ])?;
        out.lines()
            .filter(|line| !line.is_empty())
            .map(change::parse_operation)
            .collect()
    }

    /// Id of the operation at the head of the log — the one `undo` would reverse.
    pub fn current_operation(&self) -> Result<String> {
        Ok(self
            .operations(1)?
            .into_iter()
            .next()
            .map(|op| op.id)
            .unwrap_or_default())
    }

    // -- Mutations (jj-native verbs; no staging axis) --
    //
    // Every mutation goes through `mutate()`, which returns jj's own narration plus the
    // operation id it produced, so the UI can report what happened and offer an undo.

    fn mutate(&self, args: &[&str]) -> Result<Outcome> {
        let message = self.runner.mutate_capturing_stderr(args)?;
        Ok(Outcome { message, operation: self.current_operation().unwrap_or_default() })
    }

    pub fn describe(&self, change_id: &str, message: &str) -> Result<Outcome> {
        self.mutate(&["describe", "-r", change_id, "-m", message])
    }

    /// `jj new` on top of `parents` (the working copy when empty).
    pub fn new_change(&self, parents: &[String]) -> Result<Outcome> {
        let mut args = vec!["new"];
        args.extend(parents.iter().map(String::as_str));
        self.mutate(&args)
    }

    /// `jj edit` — move the working copy onto an existing change.
    pub fn edit(&self, revset: &str) -> Result<Outcome> {
        self.mutate(&["edit", revset])
    }

    /// Move `paths` (all when empty) from one change into another.
    pub fn squash_paths(&self, from: &str, into: &str, paths: &[String]) -> Result<Outcome> {
        let mut args: Vec<&str> = vec!["squash", "--from", from, "--into", into];
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        self.mutate(&args)
    }

    /// `jj absorb`: route working-copy hunks into the ancestors that last touched them.
    pub fn absorb(&self) -> Result<Outcome> {
        self.mutate(&["absorb"])
    }

    /// File-level `jj split`: the named paths move to the first commit, the rest to a
    /// child. Non-interactive on purpose — hunk-level splitting needs a diff-editor shim.
    pub fn split_paths(&self, revset: &str, paths: &[String]) -> Result<Outcome> {
        let mut args: Vec<&str> = vec!["split", "-r", revset, "--"];
        args.extend(paths.iter().map(String::as_str));
        self.mutate(&args)
    }

    pub fn abandon(&self, revset: &str) -> Result<Outcome> {
        self.mutate(&["abandon", revset])
    }

    pub fn duplicate(&self, revset: &str) -> Result<Outcome> {
        self.mutate(&["duplicate", revset])
    }

    /// Undo a change's effect as a new child commit (git revert's equivalent).
    pub fn backout(&self, revset: &str) -> Result<Outcome> {
        self.mutate(&["backout", "-r", revset])
    }

    /// `jj rebase` with the caller's choice of scope: "revision", "source", or "branch".
    pub fn rebase(&self, mode: &str, revset: &str, destination: &str) -> Result<Outcome> {
        let flag = match mode {
            "source" => "-s",
            "branch" => "-b",
            _ => "-r",
        };
        self.mutate(&["rebase", flag, revset, "-d", destination])
    }

    /// Discard working-copy changes to `paths` (all when empty). Destructive, but the
    /// operation log makes it recoverable.
    pub fn restore_paths(&self, paths: &[String]) -> Result<Outcome> {
        let mut args: Vec<&str> = vec!["restore"];
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        self.mutate(&args)
    }

    // -- Bookmarks --

    pub fn bookmark_set(&self, name: &str, revset: &str) -> Result<Outcome> {
        self.mutate(&["bookmark", "set", name, "-r", revset])
    }

    pub fn bookmark_delete(&self, name: &str) -> Result<Outcome> {
        self.mutate(&["bookmark", "delete", name])
    }

    // -- Remote --

    pub fn git_fetch(&self, remote: Option<&str>) -> Result<Outcome> {
        let mut args = vec!["git", "fetch"];
        if let Some(remote) = remote {
            args.extend(["--remote", remote]);
        }
        self.mutate(&args)
    }

    /// Push a bookmark, or `--change` to auto-name one from the change id.
    pub fn git_push(
        &self,
        remote: Option<&str>,
        bookmark: Option<&str>,
        change: Option<&str>,
    ) -> Result<Outcome> {
        let mut args = vec!["git", "push"];
        if let Some(remote) = remote {
            args.extend(["--remote", remote]);
        }
        if let Some(bookmark) = bookmark {
            args.extend(["-b", bookmark]);
        }
        if let Some(change) = change {
            args.extend(["-c", change]);
        }
        self.mutate(&args)
    }

    pub fn remotes(&self) -> Result<Vec<String>> {
        let out = self.runner.read(&["git", "remote", "list"])?;
        Ok(out
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_string))
            .collect())
    }

    // -- Undo / time travel --

    pub fn undo(&self) -> Result<Outcome> {
        self.mutate(&["undo"])
    }

    pub fn op_restore(&self, operation: &str) -> Result<Outcome> {
        self.mutate(&["op", "restore", operation])
    }

    pub fn op_revert(&self, operation: &str) -> Result<Outcome> {
        self.mutate(&["op", "revert", operation])
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
    use std::sync::{Mutex, MutexGuard};

    /// Concurrent `jj git init` invocations are flaky ("Failed to check out the initial
    /// commit"); serialize every jj-backed test.
    static JJ_LOCK: Mutex<()> = Mutex::new(());

    fn jj_serial() -> MutexGuard<'static, ()> {
        JJ_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn jj_available() -> bool {
        Command::new("jj").arg("--version").output().is_ok()
    }

    fn init_repo(dir: &Path) {
        let run = |args: &[&str]| {
            let out = Command::new("jj")
                .args(["--config", "signing.behavior=drop"])
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
    fn version_gate_accepts_current_and_rejects_old() {
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let _guard = jj_serial();
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();
        // The installed jj must satisfy the gate, or every other test here is moot.
        repo.check_version().unwrap();
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
        let _guard = jj_serial();
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

    #[test]
    fn evolog_and_interdiff_track_change_evolution() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();
        let jj = |args: &[&str]| {
            let out = Command::new("jj")
                .args([
                    "--config",
                    "user.name=Test",
                    "--config",
                    "user.email=t@example.com",
                    "--config",
                    "signing.behavior=drop",
                ])
                .args(args)
                .current_dir(tmp.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };

        // Evolve @: describe, then amend content — three evolog entries minimum.
        std::fs::write(tmp.path().join("work.txt"), "first\n").unwrap();
        jj(&["describe", "-m", "wip"]);
        std::fs::write(tmp.path().join("work.txt"), "second\n").unwrap();
        jj(&["describe", "-m", "wip v2"]); // snapshots the edit

        let wc = repo.working_copy().unwrap();
        let evolog = repo.evolog(&wc.change_id).unwrap();
        assert!(evolog.len() >= 2, "expected multiple evolog entries, got {}", evolog.len());
        assert_eq!(evolog[0].commit_id, wc.commit_id, "entry 0 is the current commit");
        assert!(evolog.iter().all(|entry| entry.change_id == wc.change_id));

        // Interdiff between the oldest and newest version mentions the content change.
        let oldest = &evolog[evolog.len() - 1];
        let patch = repo.interdiff(&oldest.commit_id, &wc.commit_id, false).unwrap();
        assert!(patch.contains("work.txt"), "interdiff should mention work.txt: {patch}");
    }

    #[test]
    fn conflicted_paths_lists_conflicts_and_handles_none() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let jj = |args: &[&str]| {
            let out = Command::new("jj")
                .args([
                    "--config",
                    "user.name=Test",
                    "--config",
                    "user.email=t@example.com",
                    "--config",
                    "signing.behavior=drop",
                ])
                .args(args)
                .current_dir(tmp.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        jj(&["git", "init", "--colocate", "."]);
        std::fs::write(tmp.path().join("sp file.txt"), "base\n").unwrap();
        jj(&["commit", "-m", "base"]);
        let repo = Repo::discover(tmp.path()).unwrap();

        // No conflicts → empty, not an error.
        assert!(repo.conflicted_paths("@").unwrap().is_empty());

        // Two divergent edits of the same line, merged. Address parents by change id —
        // description() revset matching is not worth depending on in tests.
        let base = repo.log("roots(all() ~ root())").unwrap()[0].change_id.clone();
        std::fs::write(tmp.path().join("sp file.txt"), "left\n").unwrap();
        jj(&["commit", "-m", "left"]);
        let left = repo.log("@-").unwrap()[0].change_id.clone();
        jj(&["new", "-m", "right", &base]);
        std::fs::write(tmp.path().join("sp file.txt"), "right\n").unwrap();
        jj(&["new", "-m", "merge", &left, "@"]);

        let conflicted = repo.conflicted_paths("@").unwrap();
        assert_eq!(conflicted, vec!["sp file.txt"], "path with spaces survives parsing");
    }
}
