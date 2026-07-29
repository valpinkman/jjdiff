//! Applying a *subset* of a diff's hunks to the old side of a file.
//!
//! This is the arithmetic behind hunk-level split. jj cannot select hunks
//! non-interactively — `jj split -i` hands two directories to a diff editor and
//! takes whatever the right one contains afterwards — so jjdiff plays the diff
//! editor and computes that content itself (see `src-tauri/src/split.rs`).
//!
//! Two properties make that safe to do to someone's commit:
//!
//! 1. **Every hunk is checked against the file before anything is written.** A
//!    hunk's context and removed lines must appear verbatim at the position it
//!    claims, or the diff the selection was made from no longer describes the
//!    file and the whole operation is refused.
//! 2. **Applying *all* the hunks must reproduce the new side exactly.** That is
//!    a property of any correct diff, so it costs nothing when things are well
//!    and catches everything when they are not — a stale diff, a file edited
//!    since it was read, or a hunk decomposition that does not belong to this
//!    pair of trees at all. Notably it means the hunk *boundaries* need not
//!    match the ones jj itself would have chosen: any correct cut of the same
//!    left→right change composes back to the same right.
//!
//! Offsets are all old-side, and unselected regions are copied from the old
//! side verbatim, so no offset arithmetic is needed between hunks.

use serde::{Deserialize, Serialize};

use crate::LineKind;

/// One line of a hunk in a split plan: just what applying needs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanLine {
    pub kind: LineKind,
    pub text: String,
}

/// One hunk of a split plan, and whether the reviewer picked it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanHunk {
    pub selected: bool,
    pub old_start: u32,
    pub old_lines: u32,
    pub lines: Vec<PlanLine>,
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("{path}: the hunk at old line {line} no longer matches the file — the diff moved since it was read; refresh and try again")]
    Stale { path: String, line: u32 },
    #[error("{path}: applying every hunk does not reproduce the file's current content — the diff moved since it was read; refresh and try again")]
    Mismatch { path: String },
}

/// Content for the selected side of a split: `left` with only the selected
/// hunks applied.
///
/// `right` is not the source of any output — it is the check. See the module
/// docs for why both guards are here rather than trusting the caller.
pub fn apply_selected_hunks(
    path: &str,
    left: &str,
    right: &str,
    hunks: &[PlanHunk],
) -> Result<String, ApplyError> {
    let (left_lines, left_newline) = split_lines(left);
    let (right_lines, right_newline) = split_lines(right);

    let whole = compose(path, &left_lines, hunks, |_| true)?;
    if whole.lines != right_lines {
        return Err(ApplyError::Mismatch { path: path.to_string() });
    }

    let picked = compose(path, &left_lines, hunks, |hunk| hunk.selected)?;
    // Whether the result ends in a newline is a property of whichever side its
    // last line came from: the old file when the tail was copied through, the
    // new one when a selected hunk ran to the end. Neither side records this
    // per line (`\ No newline at end of file` is dropped at parse time), and
    // guessing "always newline" would silently add a byte to a file that never
    // had one.
    let newline = if picked.tail_from_left { left_newline } else { right_newline };
    Ok(join(&picked.lines, newline))
}

struct Composed<'a> {
    lines: Vec<&'a str>,
    /// Whether any old-side line after the last applied hunk was copied through.
    tail_from_left: bool,
}

fn compose<'a>(
    path: &str,
    left: &[&'a str],
    hunks: &'a [PlanHunk],
    take: impl Fn(&PlanHunk) -> bool,
) -> Result<Composed<'a>, ApplyError> {
    let mut out: Vec<&str> = Vec::new();
    let mut cursor = 0usize;

    for hunk in hunks {
        // A pure insertion (`@@ -N,0 +M,k @@`) names the line it follows, so its
        // region starts *at* N; every other hunk names its first line, 1-based.
        let begin = if hunk.old_lines == 0 {
            hunk.old_start as usize
        } else {
            hunk.old_start.saturating_sub(1) as usize
        };
        let end = begin + hunk.old_lines as usize;
        let stale = |line: usize| ApplyError::Stale { path: path.to_string(), line: line as u32 + 1 };
        if begin < cursor || end > left.len() {
            return Err(stale(begin));
        }

        // Guard 1: the hunk's own view of the old side, line for line.
        let mut probe = begin;
        for line in &hunk.lines {
            if matches!(line.kind, LineKind::Context | LineKind::Removed) {
                if left.get(probe).copied() != Some(line.text.as_str()) {
                    return Err(stale(probe));
                }
                probe += 1;
            }
        }
        if probe != end {
            return Err(stale(begin));
        }

        if !take(hunk) {
            // Skipped: the cursor stays put, so this region is copied from the
            // old side by the next applied hunk's prefix (or by the tail).
            continue;
        }
        out.extend_from_slice(&left[cursor..begin]);
        for line in &hunk.lines {
            if !matches!(line.kind, LineKind::Removed) {
                out.push(line.text.as_str());
            }
        }
        cursor = end;
    }

    let tail_from_left = cursor < left.len();
    out.extend_from_slice(&left[cursor..]);
    Ok(Composed { lines: out, tail_from_left })
}

/// Split into lines *without* terminators, plus whether the text ended in one.
fn split_lines(text: &str) -> (Vec<&str>, bool) {
    if text.is_empty() {
        return (Vec::new(), false);
    }
    let newline = text.ends_with('\n');
    let mut lines: Vec<&str> = text.split('\n').collect();
    if newline {
        lines.pop();
    }
    (lines, newline)
}

fn join(lines: &[&str], newline: bool) -> String {
    let mut out = lines.join("\n");
    if newline && !lines.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: LineKind, text: &str) -> PlanLine {
        PlanLine { kind, text: text.to_string() }
    }

    /// Two edits at opposite ends of a file: the shape every hunk-level split
    /// is a generalisation of.
    fn two_hunks(first: bool, second: bool) -> Vec<PlanHunk> {
        vec![
            PlanHunk {
                selected: first,
                old_start: 1,
                old_lines: 2,
                lines: vec![
                    line(LineKind::Removed, "one"),
                    line(LineKind::Added, "ONE"),
                    line(LineKind::Context, "two"),
                ],
            },
            PlanHunk {
                selected: second,
                old_start: 4,
                old_lines: 2,
                lines: vec![
                    line(LineKind::Context, "four"),
                    line(LineKind::Removed, "five"),
                    line(LineKind::Added, "FIVE"),
                ],
            },
        ]
    }

    const LEFT: &str = "one\ntwo\nthree\nfour\nfive\n";
    const RIGHT: &str = "ONE\ntwo\nthree\nfour\nFIVE\n";

    #[test]
    fn selecting_one_hunk_applies_only_that_hunk() {
        let first = apply_selected_hunks("f", LEFT, RIGHT, &two_hunks(true, false)).unwrap();
        assert_eq!(first, "ONE\ntwo\nthree\nfour\nfive\n");

        let second = apply_selected_hunks("f", LEFT, RIGHT, &two_hunks(false, true)).unwrap();
        assert_eq!(second, "one\ntwo\nthree\nfour\nFIVE\n");
    }

    #[test]
    fn the_extremes_are_the_two_sides() {
        assert_eq!(apply_selected_hunks("f", LEFT, RIGHT, &two_hunks(false, false)).unwrap(), LEFT);
        assert_eq!(apply_selected_hunks("f", LEFT, RIGHT, &two_hunks(true, true)).unwrap(), RIGHT);
    }

    #[test]
    fn pure_insertions_land_after_the_line_they_name() {
        // `@@ -2,0 +3,1 @@` — inserted after old line 2.
        let hunks = vec![PlanHunk {
            selected: true,
            old_start: 2,
            old_lines: 0,
            lines: vec![line(LineKind::Added, "inserted")],
        }];
        let left = "a\nb\nc\n";
        let right = "a\nb\ninserted\nc\n";
        assert_eq!(apply_selected_hunks("f", left, right, &hunks).unwrap(), right);
    }

    #[test]
    fn an_added_file_is_one_hunk_against_nothing() {
        let hunks = vec![PlanHunk {
            selected: true,
            old_start: 0,
            old_lines: 0,
            lines: vec![line(LineKind::Added, "new")],
        }];
        assert_eq!(apply_selected_hunks("f", "", "new\n", &hunks).unwrap(), "new\n");
    }

    /// The whole point of the guard: a file edited since the diff was read must
    /// not be silently patched at the wrong offsets.
    #[test]
    fn a_hunk_that_does_not_match_the_old_side_is_refused() {
        let drifted = "ONE\ntwo\nthree\nfour\nfive\n"; // first line already changed
        let error = apply_selected_hunks("f.txt", drifted, RIGHT, &two_hunks(true, false)).unwrap_err();
        assert!(matches!(error, ApplyError::Stale { line: 1, .. }), "{error}");
        assert!(error.to_string().contains("f.txt"));
    }

    /// A diff that does not compose back to the new side is not a diff of this
    /// pair of files, whatever else it is.
    #[test]
    fn hunks_that_do_not_reproduce_the_new_side_are_refused() {
        let unrelated_right = "ONE\ntwo\nthree\nfour\nfive\nsix\n";
        let error =
            apply_selected_hunks("f.txt", LEFT, unrelated_right, &two_hunks(true, true)).unwrap_err();
        assert!(matches!(error, ApplyError::Mismatch { .. }), "{error}");
    }

    /// `\ No newline at end of file` is dropped at parse time, so the flag has
    /// to come from whichever side supplied the last line.
    #[test]
    fn trailing_newline_follows_the_side_the_tail_came_from() {
        // Old side ends without a newline; the only hunk is at the top, so the
        // tail is copied from the old side and keeps its missing newline.
        let left = "one\ntwo";
        let right = "ONE\ntwo";
        let hunks = vec![PlanHunk {
            selected: true,
            old_start: 1,
            old_lines: 1,
            lines: vec![line(LineKind::Removed, "one"), line(LineKind::Added, "ONE")],
        }];
        assert_eq!(apply_selected_hunks("f", left, right, &hunks).unwrap(), "ONE\ntwo");

        // Last hunk selected and running to the end: the new side decides, and
        // here it added the newline the old side lacked.
        let left = "one\ntwo";
        let right = "one\nTWO\n";
        let hunks = vec![PlanHunk {
            selected: true,
            old_start: 2,
            old_lines: 1,
            lines: vec![line(LineKind::Removed, "two"), line(LineKind::Added, "TWO")],
        }];
        assert_eq!(apply_selected_hunks("f", left, right, &hunks).unwrap(), "one\nTWO\n");
    }

    #[test]
    fn carriage_returns_survive_a_split() {
        let left = "one\r\ntwo\r\n";
        let right = "ONE\r\ntwo\r\n";
        let hunks = vec![PlanHunk {
            selected: true,
            old_start: 1,
            old_lines: 1,
            lines: vec![line(LineKind::Removed, "one\r"), line(LineKind::Added, "ONE\r")],
        }];
        assert_eq!(apply_selected_hunks("f", left, right, &hunks).unwrap(), right);
    }
}
