//! Diff domain for jjdiff.
//!
//! Two producers, one output shape:
//! - [`parse_git_patch`] parses `jj diff --git` output (revset diffs). Statuses and renames come
//!   from patch headers, paths from `---`/`+++` lines — never from splitting the ambiguous
//!   `diff --git a/X b/Y` header.
//! - [`worktree::diff_worktree`] diffs the live filesystem against a base tree via gix +
//!   similar, so viewing the working copy never snapshots and never writes a jj operation
//!   (PLAN.md: fs-vs-`@-`).
//!
//! Both run [`spans::add_word_spans`] so the UI gets intra-line word emphasis. Span offsets are
//! **UTF-16 code units** to match JavaScript string indexing.

mod parse;
mod spans;
pub mod worktree;

pub use parse::parse_git_patch;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePatch {
    pub path: String,
    /// Present only for renames.
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    /// Human-readable reason when contents were not diffed (e.g. too large, conflicted).
    pub skipped: Option<String>,
    /// Total added/removed line counts (file-tree badges).
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

impl FilePatch {
    fn recount(&mut self) {
        self.added = 0;
        self.removed = 0;
        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line.kind {
                    LineKind::Added => self.added += 1,
                    LineKind::Removed => self.removed += 1,
                    LineKind::Context => {}
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// Text after the second `@@`, e.g. a function name.
    pub context: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub kind: LineKind,
    pub text: String,
    /// 1-based line number on the old side (context/removed lines).
    pub old_line: Option<u32>,
    /// 1-based line number on the new side (context/added lines).
    pub new_line: Option<u32>,
    /// Intra-line emphasis ranges, `[start, end)` in UTF-16 code units.
    pub spans: Vec<(u32, u32)>,
}

impl Line {
    pub fn new(kind: LineKind, text: impl Into<String>) -> Line {
        Line { kind, text: text.into(), old_line: None, new_line: None, spans: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("malformed patch at line {line}: {message}")]
    Malformed { line: usize, message: String },
    #[error("git object access failed: {0}")]
    Gix(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
