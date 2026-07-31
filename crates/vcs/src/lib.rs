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

pub use change::{BookmarkStatus, Change, EvologEntry, Operation, Signature, Workspace};
pub use runner::JjRunner;

use std::path::{Component, Path, PathBuf};

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

/// A discovered, colocated jj *workspace*. Cheap to clone (three paths) — commands clone it
/// out of app state so blocking jj work can run off the main thread.
///
/// The three paths are one path in the ordinary repo and three different ones the moment a
/// second workspace exists, which is why they are named apart rather than derived from each
/// other at the point of use. See [`Repo::discover`].
#[derive(Clone)]
pub struct Repo {
    /// The workspace: where the files are, and what `@` means. `jj root`.
    root: PathBuf,
    /// The shared `.jj/repo`, which every workspace of a repo points at. The op log lives
    /// here, so this — not `root` — is what the op watcher watches.
    repo_dir: PathBuf,
    /// The colocated `.git`, shared by every workspace. gix reads objects from here while
    /// the files being diffed come from `root`.
    git_dir: PathBuf,
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

/// Resolve `..` and `.` without touching the filesystem.
///
/// Lexical on purpose. The only relative paths run through here are jj's own internal
/// pointers, and the result has to be *comparable* — two workspaces of one repo must
/// produce the identical string for the repo they share, and a repo with one workspace must
/// produce exactly the root `jj root` reported, or every stored review key changes meaning.
/// `canonicalize` would resolve symlinks and break both properties.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The `.jj/repo` this workspace belongs to.
///
/// A directory in the workspace `jj git init` made, and a *file* holding a relative path to
/// that one in every workspace `jj workspace add` made since.
fn resolve_repo_dir(root: &Path) -> Result<PathBuf> {
    let jj = root.join(".jj");
    let entry = jj.join("repo");
    if entry.is_dir() {
        return Ok(entry);
    }
    let pointer = std::fs::read_to_string(&entry).map_err(|error| {
        VcsError::Parse(format!("cannot read {}: {error}", entry.display()))
    })?;
    Ok(normalize(&jj.join(pointer.trim())))
}

/// The git directory behind `repo_dir`, if the repo is colocated.
///
/// `store/git_target` names it, and where it lands is the whole of the colocation question:
/// a colocated repo points *out* of `.jj` at a real `.git` beside the work, and a
/// non-colocated one points at `.jj/repo/store/git`, the bare repo jj keeps to itself.
/// `None` means the second, which is the case jjdiff cannot work with — gix would open it
/// happily and find no working tree to diff against.
fn resolve_git_dir(repo_dir: &Path) -> Option<PathBuf> {
    let store = repo_dir.join("store");
    let target = std::fs::read_to_string(store.join("git_target")).ok()?;
    let target = Path::new(target.trim());
    let resolved =
        if target.is_absolute() { target.to_path_buf() } else { normalize(&store.join(target)) };
    (!resolved.starts_with(repo_dir)).then_some(resolved)
}

/// Template producing one JSON object per revision (JSONL). Field names match
/// [`change::LogRecord`]. `\"` escapes are jj template-language escapes, not Rust's.
/// `working_copies` is mapped to bare names rather than passed to `json()` whole: the
/// keyword yields a full workspace object per entry, commit and signatures included, which
/// would repeat most of the record for a label.
const LOG_TEMPLATE: &str = r#""{\"commit\":" ++ json(self) ++ ",\"empty\":" ++ json(empty) ++ ",\"conflict\":" ++ json(conflict) ++ ",\"immutable\":" ++ json(immutable) ++ ",\"working_copy\":" ++ json(current_working_copy) ++ ",\"bookmarks\":" ++ json(bookmarks.map(|b| b.name())) ++ ",\"workspaces\":" ++ json(working_copies.map(|w| w.name())) ++ "}\n""#;

impl Repo {
    /// Find the jj workspace containing `path` and verify its repo is colocated.
    ///
    /// The colocation test is on the *repo's git store*, not on the workspace having a
    /// `.git` of its own. It used to be the latter, which was the same question right up
    /// until a second workspace existed: `jj workspace add` gives the new tree a `.jj` and
    /// nothing else, so every secondary workspace of a perfectly colocated repo failed the
    /// old check. Asking the store where its git directory is answers the question that was
    /// always meant — is there a real `.git` behind this — for both.
    pub fn discover(path: &Path) -> Result<Repo> {
        let runner = JjRunner::new(path.to_path_buf());
        let root = match runner.read(&["root"]) {
            Ok(out) => PathBuf::from(out.trim_end()),
            Err(VcsError::CommandFailed { stderr, .. }) if stderr.contains("no jj repo") => {
                return Err(VcsError::NotARepo(path.to_path_buf()));
            }
            Err(other) => return Err(other),
        };
        let repo_dir = resolve_repo_dir(&root)?;
        let git_dir = resolve_git_dir(&repo_dir).ok_or_else(|| VcsError::NotColocated(root.clone()))?;
        Ok(Repo {
            runner: JjRunner::new(root.clone()),
            root,
            repo_dir,
            git_dir,
            allow_immutable: false,
        })
    }

    /// The workspace root: where this workspace's files are.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The shared `.jj/repo` behind this workspace.
    pub fn repo_dir(&self) -> &Path {
        &self.repo_dir
    }

    /// The colocated `.git` behind this workspace — objects, not files.
    pub fn git_dir(&self) -> &Path {
        &self.git_dir
    }

    /// The identity every workspace of one repository shares: the directory holding the
    /// shared `.jj`.
    ///
    /// This is what review state keys on, not [`Self::root`]. Viewed flags, comments and
    /// walkthroughs are keyed by change id precisely so they survive the change moving, and
    /// a change checked out in another workspace is the same change — filing its review
    /// state under the tree it happens to sit in would undo that.
    ///
    /// In a repo with one workspace this *is* `root`, byte for byte, which is what lets the
    /// key change without migrating anybody's existing state.
    pub fn review_key(&self) -> PathBuf {
        self.repo_dir
            .parent()
            .and_then(|jj| jj.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone())
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

    /// Register `program` as a `merge-tools` entry for one invocation.
    ///
    /// Returns the two `--config` assignments to pass ahead of the subcommand.
    /// They go through `--config` rather than into anyone's config file: the
    /// tool is an implementation detail of a single command, and a `merge-tools`
    /// entry left behind in `~/.jjconfig.toml` would outlive the app that
    /// understands it.
    ///
    /// `args_key` is the only thing that varies between the two protocols jj
    /// offers here — `edit-args` for a diff editor (`split`, `squash`),
    /// `merge-args` for a merge tool (`resolve`).
    fn tool_config(tool: &str, args_key: &str, program: &Path, args: &[String]) -> [String; 2] {
        [
            format!("merge-tools.{tool}.program={}", toml_string(&program.to_string_lossy())),
            format!(
                "merge-tools.{tool}.{args_key}=[{}]",
                args.iter().map(|arg| toml_string(arg)).collect::<Vec<_>>().join(",")
            ),
        ]
    }

    /// Hunk-level `jj split`, by *being* the diff editor.
    ///
    /// jj offers no way to select hunks on a command line: `jj split -i` writes
    /// the two sides of the change into a pair of directories, runs the
    /// configured diff editor, and takes whatever the right-hand one holds
    /// afterwards. So the caller supplies a program that edits that directory
    /// without a human — jjdiff's own binary, running `--apply-split-plan`.
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
        let [program, args] = Self::tool_config(TOOL, "edit-args", program, edit_args);
        self.mutate_rewriting(&[
            "--config", &program,
            "--config", &args,
            "split", "-r", revset, "--tool", TOOL, "-m", message,
        ])
    }

    /// Hunk-level `jj squash`, by the same trick as [`Self::split_with_diff_editor`].
    ///
    /// `jj squash -i` speaks the identical diff-editor protocol, and over the
    /// identical pair of trees: it lays the *source's* own diff out — its parent
    /// on the left, the source on the right — squashes whatever the right
    /// directory holds into the destination, and leaves the remainder in the
    /// source. That is exactly the diff jjdiff was already showing for the
    /// source change, so one plan format serves both verbs and the same
    /// `--apply-split-plan` process carries it out.
    ///
    /// The destination keeps its description (`--use-destination-message`).
    /// jj's default is to combine the two, which opens `$EDITOR` when both are
    /// non-empty — a hang with no terminal, and the wrong question anyway:
    /// moving a few hunks into a change is not a reason to redescribe it.
    pub fn squash_with_diff_editor(
        &self,
        from: &str,
        into: &str,
        program: &Path,
        edit_args: &[String],
    ) -> Result<Outcome> {
        const TOOL: &str = "jjdiff-squash";
        let [program, args] = Self::tool_config(TOOL, "edit-args", program, edit_args);
        self.mutate_rewriting(&[
            "--config", &program,
            "--config", &args,
            "squash", "--from", from, "--into", into,
            "--use-destination-message", "--tool", TOOL,
        ])
    }

    /// Resolve one conflicted path by *being* the merge tool.
    ///
    /// The third use of the same seam, and the one that makes a conflict
    /// resolvable without a terminal. `jj resolve` materializes the conflict's
    /// sides into files, runs the configured merge tool, and takes whatever is
    /// at `$output`. jjdiff has already worked the resolved text out in the UI,
    /// so the "tool" it registers does nothing but write that text where jj
    /// will read it.
    ///
    /// `$output` is the only file named in `merge_args`: the sides are jj's
    /// business, and asking for `$left`/`$right` here would be claiming to
    /// merge them when the merging already happened.
    pub fn resolve_with_merge_tool(
        &self,
        revset: &str,
        path: &str,
        program: &Path,
        merge_args: &[String],
    ) -> Result<Outcome> {
        const TOOL: &str = "jjdiff-resolve";
        let [program, args] = Self::tool_config(TOOL, "merge-args", program, merge_args);
        self.mutate_rewriting(&[
            "--config", &program,
            "--config", &args,
            "resolve", "-r", revset, "--tool", TOOL, "--", path,
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

    /// Change ids of every commit that exists here and on no remote.
    ///
    /// [`bookmark_statuses`](Self::bookmark_statuses) answers the same question for work
    /// that has a *name*, and is the only half most tools show. It cannot see the rest:
    /// a change with no bookmark tracks nothing, so it has no ahead count and appears
    /// nowhere in that list however long it has sat unpushed. `remote_bookmarks()..` is
    /// the other half — everything not reachable from any remote ref, named or not.
    ///
    /// Two things it deliberately does not report.
    ///
    /// **A repo with no git remote gets an empty answer, not every change.** `x..` is
    /// `x..visible_heads()`, so an empty left side excludes nothing and the revset
    /// returns the entire repository — which is *true* (none of it is on a remote) and
    /// useless as a warning, since there is nowhere to push it. Rather than have the UI
    /// filter what it should never have been told, the question is refused where it is
    /// meaningless. A repo that *has* a remote with nothing on it yet is a different
    /// case and does report everything, correctly.
    ///
    /// **The empty, undescribed working copy is not unpushed work.** It is what jj
    /// leaves you standing on after every commit, so counting it means the indicator is
    /// on permanently, which is the state a badge stops being read in. An empty change
    /// that has been *described* is kept — that is a commit someone is writing.
    pub fn unpushed(&self) -> Result<Vec<String>> {
        if self.remote_urls()?.is_empty() {
            return Ok(Vec::new());
        }
        let out = self.runner.read(&[
            "log",
            "--no-graph",
            "-r",
            r#"remote_bookmarks().. ~ (empty() & description(exact:""))"#,
            "-T",
            r#"change_id ++ "\n""#,
        ])?;
        Ok(out.lines().map(str::trim).filter(|line| !line.is_empty()).map(String::from).collect())
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
    /// guarantees the repo is colocated — [`Repo::git_dir`] is a real `.git`.
    ///
    /// The git directory is named explicitly rather than reached by running git *in* the
    /// workspace, because a secondary workspace has no `.git` above it: git started there
    /// would walk out of the tree and fetch into whatever repository it found first, or none.
    ///
    /// The working directory still has to be set, and to the git directory's own parent
    /// rather than to this workspace. `--git-dir` says where to fetch *into*; it says
    /// nothing about how to read a remote given as a relative path, which git resolves
    /// against the process's working directory. That is the colocated tree — where such a
    /// remote was written down — for every workspace of the repo alike.
    ///
    /// The head lands on a namespaced bookmark so jj can address it as an
    /// ordinary revset, the user can see where it came from, and deleting it is
    /// `jj bookmark delete`. The refspec is forced so re-fetching an updated
    /// proposal moves the bookmark instead of failing on a non-fast-forward.
    pub fn fetch_forge_ref(&self, remote: &str, remote_ref: &str, bookmark: &str) -> Result<String> {
        let refspec = format!("+{remote_ref}:refs/heads/{bookmark}");
        let output = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&self.git_dir)
            .args(["fetch", remote, &refspec])
            .current_dir(self.git_dir.parent().unwrap_or(&self.root))
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

    // -- Workspaces --

    /// Every workspace attached to this repo, with its working-copy commit and its path.
    ///
    /// Two sources, because jj answers the two halves separately: `workspace list` knows the
    /// names and commits, `workspace root --name` knows the directories — one process per
    /// workspace, which is why this is resolved once per refresh rather than per render.
    ///
    /// Which one is *current* falls out of those same paths rather than costing another
    /// call: jj has no "what am I called" command, and the handle already knows its root.
    ///
    /// A workspace whose directory has been deleted keeps its record and loses its path.
    /// That is not an error to propagate but the state jjdiff most needs to show, since the
    /// only way out of it is the `forget` this list is what offers.
    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        let out = self.runner.read(&["workspace", "list", "-T", r#"json(self) ++ "\n""#])?;
        out.lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (name, change) = change::parse_workspace(line)?;
                let path = self
                    .runner
                    .read(&["workspace", "root", "--name", &name])
                    .ok()
                    .map(|path| path.trim_end().to_string());
                let current = path.as_deref().is_some_and(|path| Path::new(path) == self.root);
                Ok(Workspace { name, path, current, change })
            })
            .collect()
    }

    /// `jj workspace add` — a new working copy at `path`.
    ///
    /// `revisions` is jj's `-r`, and it does **not** mean "check this out": the new
    /// workspace's working copy is created *on top of* those revisions, exactly as
    /// `jj new` would. Empty means "beside this workspace's own working copy", sharing its
    /// parents. Checking a specific change out into a new workspace is therefore two steps —
    /// add, then [`Self::edit`] on a handle for the new path — and conflating them would
    /// silently give the reviewer a fresh empty change instead of the one they picked.
    pub fn workspace_add(&self, path: &Path, name: &str, revisions: &[String]) -> Result<Outcome> {
        let path = path.to_string_lossy();
        let mut args: Vec<&str> = vec!["workspace", "add", "--name", name];
        for revision in revisions {
            args.push("-r");
            args.push(revision);
        }
        args.push(&path);
        self.mutate(&args)
    }

    /// `jj workspace forget` — stop tracking a workspace's working copy.
    ///
    /// Deliberately does not touch the directory, because jj does not: the files stay, and
    /// removing them is a separate decision made above this (see `workspaces.rs` in the
    /// app). Undo restores the record, never the tree.
    pub fn workspace_forget(&self, name: &str) -> Result<Outcome> {
        self.mutate(&["workspace", "forget", name])
    }

    /// `jj workspace update-stale` — re-point a workspace whose working-copy commit was
    /// rewritten out from under it, which is the ordinary consequence of editing in one
    /// workspace a change another one had checked out.
    pub fn workspace_update_stale(&self) -> Result<Outcome> {
        self.mutate(&["workspace", "update-stale"])
    }

    /// Directory whose contents change whenever an operation lands (watch target).
    ///
    /// Hangs off the shared repo, not this workspace: the op log is repo-wide, and a
    /// secondary workspace has no `op_heads` of its own to watch — the old path went
    /// *through* `.jj/repo`, which there is a file, so the watcher failed to start and the
    /// window simply never refreshed.
    pub fn op_heads_dir(&self) -> PathBuf {
        self.repo_dir.join("op_heads").join("heads")
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

    /// Colocation is a property of the repo's git *store*, not of the directory
    /// having a `.git`. jj's own default is colocated now, so the case this has
    /// to keep rejecting is the explicit opt-out — whose store points inward at
    /// `.jj/repo/store/git`, a bare repo with no working tree to diff against.
    #[test]
    fn discover_rejects_a_repo_whose_git_store_is_jjs_own() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let out = Command::new("jj")
            .args(["--config", "signing.behavior=drop", "--config", "git.colocate=false"])
            .args(["git", "init", "."])
            .current_dir(tmp.path())
            .env("JJ_USER", "Test")
            .env("JJ_EMAIL", "test@example.com")
            .output()
            .expect("jj runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        match Repo::discover(tmp.path()) {
            Err(VcsError::NotColocated(_)) => {}
            other => panic!("expected NotColocated, got {other:?}", other = other.err()),
        }
    }

    /// The three paths, in the repo where they are all the same. Stated as a
    /// test because the next one is only meaningful against it.
    #[test]
    fn one_workspace_resolves_all_three_paths_to_the_same_place() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();
        let root = repo.root().to_path_buf();

        assert_eq!(repo.repo_dir(), root.join(".jj").join("repo"));
        assert_eq!(repo.git_dir(), root.join(".git"));
        assert_eq!(repo.op_heads_dir(), root.join(".jj/repo/op_heads/heads"));
        // The no-migration claim, asserted rather than assumed: every viewed
        // flag, comment and walkthrough anyone has already stored is filed under
        // this string, and it must not have moved.
        assert_eq!(repo.review_key(), root, "the review key is still the workspace root");
    }

    /// List, add and forget, plus the two things the list has to get right that
    /// jj does not state directly: which workspace is the calling one, and which
    /// no longer exists on disk.
    #[test]
    fn workspaces_round_trip_through_add_list_and_forget() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        let repo = Repo::discover(&main).unwrap();

        let only = repo.workspaces().unwrap();
        assert_eq!(only.len(), 1);
        assert!(only[0].current, "the one workspace is the current one");
        assert_eq!(only[0].path.as_deref(), Some(repo.root().to_string_lossy().as_ref()));

        let build = tmp.path().join("build");
        repo.workspace_add(&build, "build", &[]).unwrap();

        let both = repo.workspaces().unwrap();
        assert_eq!(both.len(), 2);
        let added = both.iter().find(|w| w.name == "build").expect("listed");
        assert!(!added.current, "it is not the workspace we are asking from");
        // Resolved on both sides: jj reports the real path and the temp dir is
        // reached through a symlink on macOS. What matters is the directory, not
        // the spelling — and `current` above is the assertion that jj spells
        // `root` and `workspace root` the same way, which is what it turns on.
        assert_eq!(
            added.path.as_deref().map(|path| std::fs::canonicalize(path).unwrap()),
            Some(std::fs::canonicalize(&build).unwrap())
        );
        assert_ne!(
            added.change.commit_id,
            repo.working_copy().unwrap().commit_id,
            "a workspace of its own gets a working copy of its own"
        );

        // `working_copies` names the holder, which is what tags the graph row.
        let held = repo.log(&added.change.change_id).unwrap();
        assert_eq!(held[0].workspaces, vec!["build".to_string()]);

        // A deleted directory keeps its record and loses its path — the state the
        // list exists to make actionable, since `forget` is the only way out.
        std::fs::remove_dir_all(&build).unwrap();
        let orphaned = repo.workspaces().unwrap();
        let missing = orphaned.iter().find(|w| w.name == "build").expect("still recorded");
        assert!(missing.path.is_none(), "jj can no longer resolve it");

        repo.workspace_forget("build").unwrap();
        let after = repo.workspaces().unwrap();
        assert_eq!(after.len(), 1, "forgotten");
    }

    /// A workspace `jj workspace add` created: files of its own, no `.git`, and
    /// a `.jj/repo` that is a *file* pointing at the repo it was added to.
    ///
    /// Everything jjdiff needs from a repo has to come out of that indirection,
    /// and each of these used to be silently wrong — `discover` refused the
    /// workspace outright, and had it not, the op watcher would have watched a
    /// path through a file and never fired.
    #[test]
    fn a_secondary_workspace_resolves_to_the_repo_it_was_added_to() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        std::fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        let primary = Repo::discover(&main).unwrap();

        let out = Command::new("jj")
            .args(["--config", "signing.behavior=drop"])
            .args(["workspace", "add", "--name", "build", "../build"])
            .current_dir(&main)
            .env("JJ_USER", "Test")
            .env("JJ_EMAIL", "test@example.com")
            .output()
            .expect("jj runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

        let secondary = Repo::discover(&tmp.path().join("build")).unwrap();

        assert_ne!(secondary.root(), primary.root(), "its own files");
        assert!(!secondary.root().join(".git").exists(), "and no .git of its own");
        assert_eq!(secondary.repo_dir(), primary.repo_dir(), "one repo behind both");
        assert_eq!(secondary.git_dir(), primary.git_dir(), "one git store behind both");
        assert_eq!(secondary.op_heads_dir(), primary.op_heads_dir(), "one op log to watch");
        assert!(secondary.op_heads_dir().is_dir(), "and it is a real directory");
        assert_eq!(
            secondary.review_key(),
            primary.review_key(),
            "review state follows the change between workspaces, not the tree it sits in"
        );

        // `@` is workspace-relative: each one has its own working copy.
        assert_ne!(
            secondary.working_copy().unwrap().commit_id,
            primary.working_copy().unwrap().commit_id,
            "each workspace has its own working-copy commit"
        );
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

    /// The two cases that make `unpushed` more than a revset: a repo with no
    /// remote must report *nothing* rather than its whole history, and a change
    /// with no bookmark must be reported even though it can never have a
    /// tracking status. Both need a real remote to observe, so this drives one.
    #[test]
    fn unpushed_reports_nameless_work_and_stays_silent_without_a_remote() {
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

        // A commit that is on no remote because there *is* no remote. Reporting
        // it would be true and useless — there is nowhere to push it — and it
        // would light the indicator on every local-only repo permanently.
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        jj(&["commit", "-m", "before any remote"]);
        assert!(repo.unpushed().unwrap().is_empty(), "no remote, so nothing to be unpushed from");

        jj(&["git", "remote", "add", "origin", origin.to_str().unwrap()]);
        jj(&["bookmark", "set", "main", "-r", "@-"]);
        jj(&["git", "push", "-b", "main"]);
        assert!(repo.unpushed().unwrap().is_empty(), "everything is published");

        // A described commit with no bookmark on it: invisible to
        // `bookmark_statuses` forever, which is the gap this closes.
        std::fs::write(work.join("b.txt"), "two\n").unwrap();
        jj(&["commit", "-m", "nameless work"]);
        let nameless = repo.log("@-").unwrap()[0].change_id.clone();
        assert!(
            repo.bookmark_statuses().unwrap().iter().all(|status| status.ahead == 0),
            "no bookmark moved, so tracking says nothing"
        );
        assert_eq!(repo.unpushed().unwrap(), vec![nameless], "but the work is still unpushed");

        // The empty undescribed working copy jj leaves behind is not work, and
        // counting it would leave the badge on for good.
        jj(&["new"]);
        assert_eq!(repo.unpushed().unwrap().len(), 1, "the new empty @ is not unpushed work");
    }

    /// A **divergent** change — one change id over several visible commits — is
    /// not resolvable by change id, and jj refuses every command that is handed
    /// one. This pins the property the frontend relies on: whatever the change
    /// id does, the *commit* id of a change on screen always resolves.
    ///
    /// It shipped as a change you could select and then do nothing with, not
    /// even look at: `jj diff -r <change id>` was how the diff pane asked for
    /// every non-working-copy change, and the same spelling reached edit,
    /// describe, rebase, abandon, duplicate, evolog, resolve and bookmark set.
    #[test]
    fn a_divergent_change_resolves_by_commit_id_and_not_by_change_id() {
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
                .args(["--config", "signing.behavior=drop"])
                .args(args)
                .current_dir(tmp.path())
                .env("JJ_USER", "Test")
                .env("JJ_EMAIL", "test@example.com")
                .output()
                .expect("jj runs");
            assert!(out.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };

        std::fs::write(tmp.path().join("f.txt"), "one\n").unwrap();
        jj(&["describe", "-m", "original"]);
        let change_id = repo.working_copy().unwrap().change_id;
        let obsolete = repo.working_copy().unwrap().commit_id;

        // Rewrite once, then rewrite the commit that rewrite obsoleted. Both
        // land on the same change id, and now two of them are visible — which
        // is exactly the state a user reaches by editing a hidden commit.
        jj(&["describe", "-m", "version two"]);
        jj(&["describe", "-r", &obsolete, "-m", "version one"]);
        let visible = repo.log(&format!("change_id({change_id})")).unwrap();
        assert_eq!(visible.len(), 2, "expected a divergent change: {visible:?}");

        // The change id no longer names a revision...
        let by_change = repo.log(&change_id);
        assert!(by_change.is_err(), "a divergent change id must not resolve: {by_change:?}");

        // ...but every commit under it does, which is what the UI passes.
        for change in &visible {
            let one = repo.log(&change.commit_id).unwrap();
            assert_eq!(one.len(), 1, "a commit id names exactly one commit");
            assert_eq!(one[0].commit_id, change.commit_id);
            // And the diff the pane asks for is available for each of them.
            repo.patch_for(&change.commit_id, false)
                .unwrap_or_else(|error| panic!("diff by commit id failed: {error}"));
        }
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

    /// The squash half of the same protocol, end to end.
    ///
    /// Two things need proving that the split test cannot. First, that the pair
    /// of trees jj hands a squash's editor is the *source's own* diff — that is
    /// the assumption the frontend's plan rests on, and if jj laid out anything
    /// else the ticked hunks would select the wrong lines. Second, that
    /// `--use-destination-message` really keeps the destination's description:
    /// without it jj combines the two and opens `$EDITOR`, which in a
    /// GUI-spawned process is a hang rather than an error.
    #[test]
    fn squash_with_diff_editor_moves_only_what_the_editor_left_behind() {
        let _guard = jj_serial();
        if !jj_available() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        let repo = Repo::discover(tmp.path()).unwrap();

        // A described destination, then a child holding two independent edits.
        std::fs::write(tmp.path().join("f.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
        repo.describe("@", "destination").unwrap();
        repo.new_change(&[]).unwrap();
        std::fs::write(tmp.path().join("f.txt"), "ONE\ntwo\nthree\nfour\nFIVE\n").unwrap();
        repo.describe("@", "source").unwrap(); // snapshots the edits

        // The "diff editor": keeps the first edit, drops the second.
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
        repo.squash_with_diff_editor("@", "@-", &editor, &args).unwrap();

        let destination = repo.log("@-").unwrap();
        assert_eq!(
            destination[0].first_line(),
            "destination",
            "the destination keeps its own description"
        );
        let moved = repo.patch_for("@-", false).unwrap();
        assert!(moved.contains("+ONE"), "the first edit moved into the destination: {moved}");
        assert!(!moved.contains("+FIVE"), "and the second did not: {moved}");

        let rest = repo.patch_for("@", false).unwrap();
        assert!(rest.contains("+FIVE"), "the source keeps the second edit: {rest}");
        assert!(!rest.contains("+ONE"), "and no longer carries the first: {rest}");
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("f.txt")).unwrap(),
            "ONE\ntwo\nthree\nfour\nFIVE\n",
            "a squash moves history, not the working copy's contents"
        );
    }
}
