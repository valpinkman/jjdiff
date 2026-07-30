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

pub use change::{BookmarkStatus, Change, EvologEntry, Operation, Signature};
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

/// A path with an unresolved conflict, and jj's own description of its shape
/// ("2-sided conflict", "3-sided conflict including 1 deletion", …).
///
/// The description is carried rather than discarded because it is the only
/// place the *arity* of a conflict is stated: a two-sided text conflict and a
/// three-sided one with a deletion in it want very different attention, and
/// the marker lines in the diff only say so once you are already inside them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictedFile {
    pub path: String,
    pub description: String,
}

/// A discovered, colocated jj repository. Cheap to clone (two paths) — commands clone it out
/// of app state so blocking jj work can run off the main thread.
#[derive(Clone)]
pub struct Repo {
    root: PathBuf,
    runner: JjRunner,
    /// Opt-in to rewriting commits jj has marked immutable. Off by default and
    /// never persisted — see [`Repo::allowing_immutable`].
    allow_immutable: bool,
}

/// A TOML basic string. `--config` values are parsed as TOML, so anything going
/// through one has to survive that pass — a Windows path or a temp directory
/// with a quote in it would otherwise turn into a config syntax error at best.
fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
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
        Ok(Repo { runner: JjRunner::new(root.clone()), root, allow_immutable: false })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A handle whose *rewriting* commands carry `--ignore-immutable`.
    ///
    /// Deliberately per-call rather than a mode on the app: nothing stores it, so
    /// the override lasts exactly one command and the next one is gated again.
    /// jj marks commits immutable to stop precisely this happening by accident,
    /// and a sticky "allow immutable" toggle would hand that guarantee back for
    /// the rest of a session.
    pub fn allowing_immutable(&self, allow: bool) -> Repo {
        Repo { allow_immutable: allow, ..self.clone() }
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

    /// Unresolved conflicts in `revset`. Parses `jj resolve --list` lines of the
    /// form `<path>    <N>-sided conflict…` — the split is made from the *right*
    /// so paths containing spaces survive, and both halves are kept.
    pub fn conflicts(&self, revset: &str) -> Result<Vec<ConflictedFile>> {
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
                Some(position) => ConflictedFile {
                    path: line[..position].trim_end().to_string(),
                    description: line[position..].trim().to_string(),
                },
                None => ConflictedFile {
                    path: line.trim_end().to_string(),
                    description: String::new(),
                },
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

    /// What an operation actually did, as jj's own narration: the commits it changed, where
    /// the working copy moved, which bookmarks shifted. `from` is the earlier operation; with
    /// `None`, `to` is compared against its own parent ("what did this one do").
    ///
    /// This is the one read that returns text rather than a structure, and deliberately so:
    /// `jj op diff` accepts no `-T`, and it has no `json()` form to ask for. Parsing its
    /// prose would be exactly what the templates invariant exists to prevent, so the string
    /// is passed through for display — the same contract as a mutation's narration.
    pub fn op_diff(&self, from: Option<&str>, to: &str) -> Result<String> {
        let args = match from {
            Some(from) => vec!["op", "diff", "--no-graph", "--from", from, "--to", to],
            None => vec!["op", "diff", "--no-graph", "--operation", to],
        };
        self.runner.read(&args)
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

    /// [`Self::mutate`] for verbs that *rewrite an existing commit*, which are the
    /// only ones jj's immutability check applies to — and the only ones that accept
    /// `--ignore-immutable`. `backout` and `duplicate` reject the flag outright
    /// (they add commits rather than rewrite them), so routing everything through
    /// one helper would turn an unrelated command into a parse error.
    ///
    /// The flag goes before the subcommand: that is the position jj accepts on
    /// every rewriting verb, and it matches how `--ignore-working-copy` is passed
    /// on the read path.
    fn mutate_rewriting(&self, args: &[&str]) -> Result<Outcome> {
        if !self.allow_immutable {
            return self.mutate(args);
        }
        let mut full = vec!["--ignore-immutable"];
        full.extend_from_slice(args);
        self.mutate(&full)
    }

    pub fn describe(&self, change_id: &str, message: &str) -> Result<Outcome> {
        self.mutate_rewriting(&["describe", "-r", change_id, "-m", message])
    }

    /// `jj new` on top of `parents` (the working copy when empty).
    pub fn new_change(&self, parents: &[String]) -> Result<Outcome> {
        let mut args = vec!["new"];
        args.extend(parents.iter().map(String::as_str));
        self.mutate(&args)
    }

    /// `jj edit` — move the working copy onto an existing change.
    pub fn edit(&self, revset: &str) -> Result<Outcome> {
        self.mutate_rewriting(&["edit", revset])
    }

    /// Move `paths` (all when empty) from one change into another.
    pub fn squash_paths(&self, from: &str, into: &str, paths: &[String]) -> Result<Outcome> {
        let mut args: Vec<&str> = vec!["squash", "--from", from, "--into", into];
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        self.mutate_rewriting(&args)
    }

    /// `jj absorb`: route working-copy hunks into the ancestors that last touched them.
    pub fn absorb(&self) -> Result<Outcome> {
        self.mutate_rewriting(&["absorb"])
    }

    /// File-level `jj split`: the named paths move to the first commit, the rest to a
    /// child. See [`Self::split_with_diff_editor`] for the hunk-level form.
    pub fn split_paths(&self, revset: &str, paths: &[String]) -> Result<Outcome> {
        let mut args: Vec<&str> = vec!["split", "-r", revset, "--"];
        args.extend(paths.iter().map(String::as_str));
        self.mutate_rewriting(&args)
    }

    /// Hunk-level `jj split`, by *being* the diff editor.
    ///
    /// jj offers no way to select hunks on a command line: `jj split -i` writes
    /// the two sides of the change into a pair of directories, runs the
    /// configured diff editor, and takes whatever the right-hand one holds
    /// afterwards. So the caller supplies a program that edits that directory
    /// without a human — jjdiff's own binary, running `--apply-split-plan`.
    ///
    /// The tool is registered through `--config` for this one invocation rather
    /// than written into anyone's config file: it is an implementation detail of
    /// one command, and a `merge-tools` entry left behind in `~/.jjconfig.toml`
    /// would outlive the app that understands it.
    ///
    /// `message` describes the selected half. Passing it is what keeps the whole
    /// thing non-interactive — without `-m`, jj opens `$EDITOR` for a
    /// description and a GUI-spawned editor with no terminal simply hangs.
    pub fn split_with_diff_editor(
        &self,
        revset: &str,
        program: &Path,
        edit_args: &[String],
        message: &str,
    ) -> Result<Outcome> {
        const TOOL: &str = "jjdiff-split";
        let program = format!(
            "merge-tools.{TOOL}.program={}",
            toml_string(&program.to_string_lossy())
        );
        let args = format!(
            "merge-tools.{TOOL}.edit-args=[{}]",
            edit_args.iter().map(|arg| toml_string(arg)).collect::<Vec<_>>().join(",")
        );
        self.mutate_rewriting(&[
            "--config", &program,
            "--config", &args,
            "split", "-r", revset, "--tool", TOOL, "-m", message,
        ])
    }

    pub fn abandon(&self, revset: &str) -> Result<Outcome> {
        self.mutate_rewriting(&["abandon", revset])
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
        self.mutate_rewriting(&["rebase", flag, revset, "-d", destination])
    }

    /// Discard working-copy changes to `paths` (all when empty). Destructive, but the
    /// operation log makes it recoverable.
    pub fn restore_paths(&self, paths: &[String]) -> Result<Outcome> {
        let mut args: Vec<&str> = vec!["restore"];
        if !paths.is_empty() {
            args.push("--");
            args.extend(paths.iter().map(String::as_str));
        }
        self.mutate_rewriting(&args)
    }

    // -- Bookmarks --

    /// Ahead/behind for every local bookmark that tracks a remote.
    ///
    /// Two traps, both encoded here rather than at the call site:
    ///
    /// 1. **The counts are inverted.** The keywords live on the *remote* ref and
    ///    describe the remote's position, so `tracking_behind_count` — the remote
    ///    lagging — is the local bookmark being *ahead*. Reading them straight
    ///    produces a display that is exactly backwards, and plausibly so.
    /// 2. **The `git` remote is not a remote.** Colocated repos carry a synthetic
    ///    `git` remote mirroring the git HEAD refs; it is always in sync by
    ///    construction, so reporting it is noise that dilutes the real answer.
    ///
    /// Untracked and local-only bookmarks are omitted — there is no remote to be
    /// ahead of, and the count keywords error rather than return zero.
    pub fn bookmark_statuses(&self) -> Result<Vec<BookmarkStatus>> {
        const BOOKMARK_TEMPLATE: &str = r#"if(remote && tracked, "{\"name\":" ++ json(name) ++ ",\"remote\":" ++ json(remote) ++ ",\"ahead\":" ++ tracking_behind_count.lower() ++ ",\"behind\":" ++ tracking_ahead_count.lower() ++ "}\n")"#;
        let out = self.runner.read(&[
            "bookmark", "list", "--all-remotes", "-T", BOOKMARK_TEMPLATE,
        ])?;
        let mut statuses: Vec<BookmarkStatus> = out
            .lines()
            .filter(|line| !line.is_empty())
            .map(change::parse_bookmark_status)
            .collect::<Result<_>>()?;
        statuses.retain(|status| status.remote != "git");
        Ok(statuses)
    }

    pub fn bookmark_set(&self, name: &str, revset: &str) -> Result<Outcome> {
        self.mutate(&["bookmark", "set", name, "-r", revset])
    }

    pub fn bookmark_delete(&self, name: &str) -> Result<Outcome> {
        self.mutate(&["bookmark", "delete", name])
    }

    // -- Remote --

    /// Fetch one bookmark. Reviewing a proposal needs the base branch present
    /// locally — the forge's merge-base commit is an ancestor of it, and
    /// without it the review revset cannot resolve.
    pub fn git_fetch_branch(&self, remote: &str, branch: &str) -> Result<Outcome> {
        self.mutate(&["git", "fetch", "--remote", remote, "--branch", branch])
    }

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

    /// Fetch a forge proposal's head into a local bookmark, returning it.
    ///
    /// This is the one place jjdiff shells out to **git** rather than jj, and
    /// the reason is structural: proposal heads live outside `refs/heads/*`
    /// (`refs/pull/N/head`, `refs/merge-requests/N/head`) and `jj git fetch`
    /// takes bookmark globs, not refspecs. Safe because [`Repo::discover`]
    /// guarantees the repo is colocated — the `.git` directory is right there.
    ///
    /// The head lands on a namespaced bookmark so jj can address it as an
    /// ordinary revset, the user can see where it came from, and deleting it is
    /// `jj bookmark delete`. The refspec is forced so re-fetching an updated
    /// proposal moves the bookmark instead of failing on a non-fast-forward.
    pub fn fetch_forge_ref(&self, remote: &str, remote_ref: &str, bookmark: &str) -> Result<String> {
        let refspec = format!("+{remote_ref}:refs/heads/{bookmark}");
        let output = std::process::Command::new("git")
            .args(["fetch", remote, &refspec])
            .current_dir(&self.root)
            .output()
            .map_err(VcsError::Io)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(VcsError::CommandFailed {
                args: vec!["git".into(), "fetch".into(), remote.into(), refspec],
                stderr,
            });
        }
        // Colocated repos import on the next jj command anyway, but doing it
        // here means the bookmark is addressable the moment this returns.
        self.runner.mutate(&["git", "import"])?;
        Ok(bookmark.to_string())
    }

    /// Remotes as `(name, url)`. `jj git remote list` prints them space
    /// separated; the URL is everything after the first field, so a URL
    /// containing spaces survives.
    pub fn remote_urls(&self) -> Result<Vec<(String, String)>> {
        let out = self.runner.read(&["git", "remote", "list"])?;
        Ok(out
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| {
                let (name, url) = line.split_once(char::is_whitespace)?;
                Some((name.to_string(), url.trim().to_string()))
            })
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
        // Pin the signing behaviour *in the repo*, not just on the invocations
        // this helper makes. Tests drive `Repo` too, and `Repo` deliberately
        // runs jj the way the user's own jj runs — so without this the suite
        // inherits whatever `signing.backend` the machine has configured, and
        // fails with "agent refused operation" whenever the ssh-agent is locked
        // or wants a touch. That is a real failure of the test environment
        // masquerading as a failure of the code.
        run(&["config", "set", "--repo", "signing.behavior", "drop"]);
        // `signing.behavior` does not cover the push path: `git.sign-on-push`
        // signs at push time regardless, so a test that pushes still hits the
        // agent without this.
        run(&["config", "set", "--repo", "git.sign-on-push", "false"]);
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
    fn op_diff_narrates_a_single_operation_and_a_range() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();

        repo.describe("@", "op diff subject").unwrap();
        repo.new_change(&[]).unwrap();

        let ops = repo.operations(10).unwrap();
        let head = &ops[0];
        assert_eq!(head.description, "new empty commit");

        // A single operation, against its own parent.
        let one = repo.op_diff(None, &head.id).unwrap();
        assert!(one.contains("Changed commits"), "expected a commit section: {one}");

        // A range spanning both mutations mentions the description the first one set.
        let older = ops.iter().find(|op| op.description.starts_with("describe")).unwrap();
        let range = repo.op_diff(Some(&older.id), &head.id).unwrap();
        assert!(range.contains("op diff subject"), "range should span the describe: {range}");

        // Reads never write: asking twice does not add an operation.
        assert_eq!(repo.operations(10).unwrap().len(), ops.len());
    }

    #[test]
    fn fetch_forge_ref_lands_a_pull_head_on_an_addressable_bookmark() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();

        // Stand in for a forge: publish the initial commit under the same
        // `refs/pull/N/head` namespace GitHub uses, then fetch from ourselves.
        let head = repo.log("@-").unwrap()[0].commit_id.clone();
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(tmp.path())
                .output()
                .expect("git runs");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(&["update-ref", "refs/pull/7/head", &head]);

        let bookmark = repo.fetch_forge_ref(".", "refs/pull/7/head", "jjdiff-pr-7").unwrap();
        assert_eq!(bookmark, "jjdiff-pr-7");

        // The point of the exercise: the head is now an ordinary revset.
        let fetched = repo.log("jjdiff-pr-7").unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].commit_id, head);

        // Re-fetching an updated proposal must move the bookmark, not fail on
        // a non-fast-forward — hence the forced refspec.
        std::fs::write(tmp.path().join("hello.txt"), "moved\n").unwrap();
        Command::new("jj")
            .args(["--config", "signing.behavior=drop", "commit", "-m", "pr update"])
            .current_dir(tmp.path())
            .env("JJ_USER", "Test")
            .env("JJ_EMAIL", "test@example.com")
            .output()
            .unwrap();
        let moved = repo.log("@-").unwrap()[0].commit_id.clone();
        git(&["update-ref", "refs/pull/7/head", &moved]);
        repo.fetch_forge_ref(".", "refs/pull/7/head", "jjdiff-pr-7").unwrap();
        assert_eq!(repo.log("jjdiff-pr-7").unwrap()[0].commit_id, moved);
    }

    #[test]
    fn fetch_forge_ref_reports_a_missing_ref() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();
        // A proposal number that does not exist must surface git's own words,
        // not a silent empty bookmark.
        let error = repo.fetch_forge_ref(".", "refs/pull/999/head", "jjdiff-pr-999").unwrap_err();
        assert!(
            matches!(error, VcsError::CommandFailed { .. }),
            "expected CommandFailed, got {error:?}"
        );
    }

    /// The counts are inverted relative to the template keywords they come from,
    /// so a plausible-looking implementation reports the exact opposite. This
    /// drives both directions against a real remote for that reason — a unit test
    /// on a fixture could not have caught it.
    #[test]
    fn bookmark_statuses_report_ahead_and_behind_from_the_local_side() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        git(tmp.path(), &["init", "--bare", "-q", "origin.git"]);
        init_repo(&work);
        let jj = |args: &[&str]| {
            let out = Command::new("jj")
                .args(["--config", "signing.behavior=drop"])
                .args(args)
                .current_dir(&work)
                .env("JJ_USER", "Test")
                .env("JJ_EMAIL", "test@example.com")
                .output()
                .expect("jj runs");
            assert!(out.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let repo = Repo::discover(&work).unwrap();

        // No remotes yet: an empty list, not an error.
        assert!(repo.bookmark_statuses().unwrap().is_empty());

        jj(&["bookmark", "set", "main", "-r", "@-"]);
        jj(&["git", "remote", "add", "origin", origin.to_str().unwrap()]);
        jj(&["git", "push", "-b", "main"]);

        // In sync — and the synthetic `git` remote of a colocated repo must not
        // show up at all.
        let synced = repo.bookmark_statuses().unwrap();
        assert_eq!(synced.len(), 1, "only the real remote: {synced:?}");
        assert_eq!(synced[0].remote, "origin");
        assert_eq!((synced[0].ahead, synced[0].behind), (0, 0));

        // Two local commits the remote has never seen → ahead by 2.
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        jj(&["commit", "-m", "local one"]);
        std::fs::write(work.join("b.txt"), "two\n").unwrap();
        jj(&["commit", "-m", "local two"]);
        jj(&["bookmark", "set", "main", "-r", "@-"]);
        let ahead = repo.bookmark_statuses().unwrap();
        assert_eq!((ahead[0].ahead, ahead[0].behind), (2, 0), "local is ahead: {ahead:?}");

        // Publish, then rewind the local bookmark one commit → behind by 1.
        jj(&["git", "push", "-b", "main"]);
        jj(&["bookmark", "set", "main", "-r", "main@origin-", "--allow-backwards"]);
        let behind = repo.bookmark_statuses().unwrap();
        assert_eq!((behind[0].ahead, behind[0].behind), (0, 1), "local is behind: {behind:?}");
    }

    /// `--ignore-immutable` is opt-in per handle, and the default handle must
    /// still be refused by jj — otherwise the confirmation in the UI is
    /// decoration over an override that was always on.
    #[test]
    fn immutable_commits_are_rewritable_only_through_the_opt_in_handle() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();

        // Pin an immutable set that is not just the root commit: everything at or
        // below the initial commit, which is what `trunk()` would give in a repo
        // with a real main bookmark.
        let target = repo.log("@-").unwrap()[0].change_id.clone();
        let out = Command::new("jj")
            // The parenthesised alias name has to be a quoted TOML key.
            .args(["config", "set", "--repo", "revset-aliases.'immutable_heads()'", "@-"])
            .current_dir(tmp.path())
            .output()
            .expect("jj runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(repo.log("@-").unwrap()[0].immutable, "test setup: @- must be immutable");

        // Default handle: jj refuses, and says why.
        let error = repo.describe(&target, "rewritten").unwrap_err();
        match &error {
            VcsError::CommandFailed { stderr, .. } => {
                assert!(stderr.contains("immutable"), "expected an immutability error, got {stderr}");
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }

        // Opt-in handle: the same call lands.
        repo.allowing_immutable(true).describe(&target, "rewritten").unwrap();
        assert_eq!(repo.log(&target).unwrap()[0].first_line(), "rewritten");

        // And the opt-in does not stick to the original handle.
        assert!(repo.describe(&target, "again").is_err(), "the override must be per-handle");
    }

    #[test]
    fn toml_strings_survive_the_config_parser() {
        assert_eq!(toml_string("/tmp/plain"), "\"/tmp/plain\"");
        assert_eq!(toml_string(r#"C:\dir\"x""#), r#""C:\\dir\\\"x\"""#);
        assert_eq!(toml_string("a\tb"), "\"a\\tb\"");
    }

    #[test]
    fn conflicts_list_paths_with_their_shape_and_handle_none() {
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
        assert!(repo.conflicts("@").unwrap().is_empty());

        // Two divergent edits of the same line, merged. Address parents by change id —
        // description() revset matching is not worth depending on in tests.
        let base = repo.log("roots(all() ~ root())").unwrap()[0].change_id.clone();
        std::fs::write(tmp.path().join("sp file.txt"), "left\n").unwrap();
        jj(&["commit", "-m", "left"]);
        let left = repo.log("@-").unwrap()[0].change_id.clone();
        jj(&["new", "-m", "right", &base]);
        std::fs::write(tmp.path().join("sp file.txt"), "right\n").unwrap();
        jj(&["new", "-m", "merge", &left, "@"]);

        let conflicted = repo.conflicts("@").unwrap();
        assert_eq!(conflicted.len(), 1);
        assert_eq!(conflicted[0].path, "sp file.txt", "path with spaces survives parsing");
        assert!(
            conflicted[0].description.contains("sided conflict"),
            "jj's own description of the conflict is kept: {:?}",
            conflicted[0].description
        );
    }

    /// The whole hunk-level split, end to end against a real `jj split`: a
    /// scripted diff editor is the only way to select part of a change, and
    /// nothing short of running one proves the protocol is being spoken.
    #[test]
    fn split_with_diff_editor_keeps_only_what_the_editor_left_behind() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();

        // A change with two independent edits, at opposite ends of one file.
        std::fs::write(tmp.path().join("f.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        repo.describe("@", "both edits").unwrap();
        std::fs::write(tmp.path().join("f.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
        repo.describe("@", "both edits").unwrap(); // snapshots the edit

        // The "diff editor": writes the first edit alone into the right dir.
        let editor = tmp.path().join("editor.sh");
        std::fs::write(
            &editor,
            "#!/bin/sh\nprintf 'ONE\\ntwo\\nthree\\nfour\\nfive\\n' > \"$2/f.txt\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let args = vec!["$left".to_string(), "$right".to_string()];
        repo.split_with_diff_editor("@", &editor, &args, "first edit only").unwrap();

        // Two commits now, and the earlier one carries only the first edit.
        let selected = repo.log("@-").unwrap();
        assert_eq!(selected[0].first_line(), "first edit only");
        let patch = repo.patch_for("@-", false).unwrap();
        assert!(patch.contains("+ONE"), "the selected half has the first edit: {patch}");
        assert!(!patch.contains("+FIVE"), "and not the second: {patch}");

        // The remainder keeps the rest, and the two together are the original.
        let rest = repo.patch_for("@", false).unwrap();
        assert!(rest.contains("+FIVE"), "the remainder has the second edit: {rest}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("f.txt")).unwrap(),
            "ONE\ntwo\nthree\nfour\nFIVE\n",
            "the working copy is unchanged by a split"
        );
    }
}
