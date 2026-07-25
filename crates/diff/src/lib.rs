//! Diff domain for jjdiff.
//!
//! M0: parse `jj diff --git` output into structured data for the UI. Statuses and renames come
//! from patch headers (`new file mode`, `deleted file mode`, `rename from`/`rename to`), which
//! are unambiguous even for paths with spaces — unlike `--summary`'s `R {old => new}` braces.
//! Paths for plain edits come from the `---`/`+++` lines, not from splitting the ambiguous
//! `diff --git a/X b/Y` header.
//!
//! M1 adds fs-vs-tree diffing (gix + imara-diff) so working-copy views never snapshot.

mod parse;

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
    pub hunks: Vec<Hunk>,
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
}
