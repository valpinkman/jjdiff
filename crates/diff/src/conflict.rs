//! Materialized jj conflicts, taken apart into something a UI can offer choices about.
//!
//! jj stores conflicts *inside* commits and hands them to you as marker text —
//! the same text `jj file show` prints and the same text sits in the working
//! copy. Reading it is fine; picking a side out of it by hand is not, and that
//! is the whole reason resolving stayed terminal-only. So this module reads the
//! markers back into structure: a file becomes a run of ordinary text and
//! [`Conflict`] regions, each region a list of named sides, and choosing one is
//! then an ordinary edit rather than a careful deletion of six fences.
//!
//! Three marker styles are understood, because jj emits whichever
//! `conflict-marker-style` says and a repo that has been through a git merge
//! tool can hold the fourth:
//!
//! - **diff** (jj's default) — `%%%%%%%` introduces a side written as a diff
//!   from the base, `+++++++` a side written out whole.
//! - **snapshot** — every side under `+++++++`, the base under `-------`.
//! - **git** — `|||||||` for the base, `=======` between the two sides.
//!
//! The base is kept rather than discarded even though it is rarely the answer:
//! it is the only thing that says what the two sides were both changing, and a
//! three-way conflict is unreadable without it.
//!
//! Markers are matched at **seven characters or more**. jj lengthens them when
//! the conflicted content itself contains something that would otherwise close
//! the region, so a fixed length of seven would mis-parse exactly the files
//! that need the care.

use serde::Serialize;

/// One side of a conflict: jj's own label for it, and its content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictSide {
    /// Whatever jj wrote after the marker — for a jj conflict this names the
    /// commit and its description, which is why these are not relabelled here.
    pub label: String,
    pub lines: Vec<String>,
}

/// One `<<<<<<<` … `>>>>>>>` region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// Position among the file's conflicts, 0-based. The id the UI addresses a
    /// region by, and stable only for one reading of one file.
    pub index: usize,
    /// The text on the opening fence (`Conflict 1 of 2`).
    pub label: String,
    pub sides: Vec<ConflictSide>,
    /// The merge base, when the markers stated one. Diff-style conflicts always
    /// do (it is what the diff is *from*); git-style ones only with
    /// `diff3`/`zdiff3` set.
    pub base: Option<ConflictSide>,
}

/// A file split into the parts that are agreed and the parts that are not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Piece {
    /// Lines outside any conflict, verbatim.
    Text { lines: Vec<String> },
    Conflict(Conflict),
}

/// A materialized conflicted file, ready to be resolved region by region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictedContent {
    pub pieces: Vec<Piece>,
    /// Whether the file ended with a newline. Carried because the resolution is
    /// reassembled from lines, and a file that silently grows or loses its final
    /// newline shows up as a spurious diff on the next read.
    pub trailing_newline: bool,
}

impl ConflictedContent {
    /// How many conflict regions the file holds. One file routinely holds
    /// several, which is why the banner counts files and this counts regions.
    pub fn conflict_count(&self) -> usize {
        self.pieces.iter().filter(|piece| matches!(piece, Piece::Conflict(_))).count()
    }
}

/// A marker line: its kind, how long its run of fence characters was, and the
/// label that followed it.
///
/// The run length is not decoration. jj picks one marker length per conflict,
/// long enough to exceed anything conflict-shaped in the content, and writes
/// every fence of that region at that length — so inside a region opened with
/// nine characters, a line of seven is *content*, and a parser that does not
/// carry the opening length cannot tell.
fn marker(line: &str, min_run: usize) -> Option<(Marker, usize, String)> {
    let first = line.chars().next()?;
    let kind = match first {
        '<' => Marker::Start,
        '>' => Marker::End,
        '+' => Marker::Side,
        '%' => Marker::DiffSide,
        '-' => Marker::Base,
        '|' => Marker::Base,
        '=' => Marker::GitSeparator,
        _ => return None,
    };
    let run = line.chars().take_while(|c| *c == first).count();
    if run < min_run {
        return None;
    }
    // The rest of the line is jj's label. A marker with no label is legal (git
    // writes a bare `=======`), and a run followed by anything other than a
    // space is not a marker at all — `++++++++x` is content.
    let rest = &line[first.len_utf8() * run..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((kind, run, rest.trim().to_string()))
}

/// The shortest run any marker can have. jj never goes below it, and git's are
/// exactly this long.
const MIN_MARKER: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    Start,
    End,
    /// `+++++++` — a side given verbatim.
    Side,
    /// `%%%%%%%` — a side given as a diff from the base.
    DiffSide,
    /// `-------` (snapshot) or `|||||||` (git) — the base, verbatim.
    Base,
    /// `=======` — git's separator, which both closes one side and opens the next.
    GitSeparator,
}

/// Take a materialized conflicted file apart.
///
/// Never fails: text that does not parse as a conflict is text. An unterminated
/// region — a truncated file, or a `<<<<<<<` that is genuinely content — comes
/// back as the ordinary lines it is made of rather than as a half-region,
/// because the alternative is a resolution that silently drops the tail of the
/// file.
pub fn parse_conflicts(content: &str) -> ConflictedContent {
    let trailing_newline = content.ends_with('\n');
    let lines: Vec<&str> = split_lines(content);

    let mut pieces: Vec<Piece> = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut index = 0usize;
    let mut cursor = 0usize;

    while cursor < lines.len() {
        let opened =
            marker(lines[cursor], MIN_MARKER).filter(|(kind, _, _)| *kind == Marker::Start);
        let Some((_, run, label)) = opened else {
            text.push(lines[cursor].to_string());
            cursor += 1;
            continue;
        };
        match parse_region(&lines, cursor, index, label, run) {
            Some((conflict, next)) => {
                if !text.is_empty() {
                    pieces.push(Piece::Text { lines: std::mem::take(&mut text) });
                }
                pieces.push(Piece::Conflict(conflict));
                index += 1;
                cursor = next;
            }
            // Unterminated: keep the fence as the content it evidently is.
            None => {
                text.push(lines[cursor].to_string());
                cursor += 1;
            }
        }
    }
    if !text.is_empty() {
        pieces.push(Piece::Text { lines: text });
    }
    ConflictedContent { pieces, trailing_newline }
}

/// Parse one region starting at `start` (the `<<<<<<<` line). Returns the
/// region and the index just past its `>>>>>>>`, or `None` if it never closes.
///
/// `run` is the opening fence's length: inside this region nothing shorter is
/// a marker, which is what lets a conflict whose content contains fences be
/// read back correctly.
fn parse_region(
    lines: &[&str],
    start: usize,
    index: usize,
    label: String,
    run: usize,
) -> Option<(Conflict, usize)> {
    let mut sides: Vec<ConflictSide> = Vec::new();
    let mut base: Option<ConflictSide> = None;
    // git style opens its first side immediately after `<<<<<<<`, with no
    // marker of its own; jj styles always announce each section. `Leading`
    // covers both by being the one section that is dropped when it is empty —
    // an announced section that is empty is a side that deletes everything,
    // which is a real answer and must survive.
    let mut section = Section::Leading(ConflictSide { label: label.clone(), lines: Vec::new() });

    let mut cursor = start + 1;
    while cursor < lines.len() {
        let line = lines[cursor];
        match marker(line, run) {
            Some((Marker::End, _, end_label)) => {
                section.flush(&mut sides, &mut base);
                // A region with nothing in it is not a conflict jj could have
                // written; treat the fence as content rather than inventing a
                // conflict with no sides for the UI to offer.
                if sides.is_empty() {
                    return None;
                }
                // git names the second side on the closing fence, not the
                // separator, so the label arrives after the content it belongs
                // to. jj labels every section as it opens one, and never
                // reaches this.
                if let Some(last) = sides.last_mut() {
                    if last.label.is_empty() {
                        last.label = end_label;
                    }
                }
                return Some((Conflict { index, label, sides, base }, cursor + 1));
            }
            Some((Marker::Start, _, _)) => {
                // Nested opener at this region's own fence length: the region
                // never closes before another begins.
                return None;
            }
            Some((kind, _, section_label)) => {
                section.flush(&mut sides, &mut base);
                section = match kind {
                    Marker::Side => Section::Side(ConflictSide { label: section_label, lines: Vec::new() }),
                    Marker::DiffSide => Section::Diff {
                        label: section_label,
                        base: Vec::new(),
                        side: Vec::new(),
                    },
                    Marker::Base => Section::Base(ConflictSide { label: section_label, lines: Vec::new() }),
                    Marker::GitSeparator => {
                        Section::Side(ConflictSide { label: section_label, lines: Vec::new() })
                    }
                    Marker::Start | Marker::End => unreachable!("handled above"),
                };
            }
            None => section.push(line),
        }
        cursor += 1;
    }
    None
}

/// The section currently being read. Diff sections are the only ones that
/// contribute to two places at once — they state the base and a side together,
/// which is exactly what makes jj's default style readable and this parser
/// worth having.
enum Section {
    /// Content between `<<<<<<<` and the first section marker. A side in git
    /// style, nothing at all in jj's.
    Leading(ConflictSide),
    Side(ConflictSide),
    Base(ConflictSide),
    Diff { label: String, base: Vec<String>, side: Vec<String> },
}

impl Section {
    fn push(&mut self, line: &str) {
        match self {
            Section::Leading(side) | Section::Side(side) | Section::Base(side) => {
                side.lines.push(line.to_string())
            }
            Section::Diff { base, side, .. } => {
                // A diff line's first character is its operation. An empty line
                // inside a diff section is an empty context line whose single
                // space some tool trimmed; treating it as context is the only
                // reading that keeps both sides the right length.
                let (operation, rest) = match line.chars().next() {
                    Some(first @ ('+' | '-' | ' ')) => (first, &line[first.len_utf8()..]),
                    _ => (' ', line),
                };
                match operation {
                    '-' => base.push(rest.to_string()),
                    '+' => side.push(rest.to_string()),
                    _ => {
                        base.push(rest.to_string());
                        side.push(rest.to_string());
                    }
                }
            }
        }
    }

    /// File the finished section, dropping the empty placeholder git-style
    /// parsing starts with.
    fn flush(&mut self, sides: &mut Vec<ConflictSide>, base_out: &mut Option<ConflictSide>) {
        let done = std::mem::replace(
            self,
            Section::Leading(ConflictSide { label: String::new(), lines: Vec::new() }),
        );
        match done {
            Section::Leading(side) => {
                if !side.lines.is_empty() {
                    sides.push(side);
                }
            }
            Section::Side(side) => sides.push(side),
            Section::Base(side) => {
                if base_out.is_none() {
                    *base_out = Some(side);
                }
            }
            Section::Diff { label, base, side } => {
                if base_out.is_none() {
                    *base_out = Some(ConflictSide { label: "base".to_string(), lines: base });
                }
                sides.push(ConflictSide { label, lines: side });
            }
        }
    }
}

/// Split on `\n`, dropping the empty final element a trailing newline produces.
/// `trailing_newline` carries that fact instead, so a round trip through
/// [`parse_conflicts`] and back reproduces the file byte for byte.
fn split_lines(content: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    if content.is_empty() {
        lines.clear();
    }
    lines
}

/// Whether `content` still holds a conflict fence.
///
/// The gate on a resolution. jj takes whatever the merge tool leaves at
/// `$output` at face value — with `merge-tool-edits-conflict-markers` unset it
/// does not re-parse markers — so handing it text that still has fences in it
/// writes seven angle brackets into the file and calls the conflict resolved.
/// That is worth refusing rather than discovering later.
pub fn has_conflict_markers(content: &str) -> bool {
    split_lines(content)
        .iter()
        .any(|line| matches!(marker(line, MIN_MARKER), Some((Marker::Start | Marker::End, _, _))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflicts(parsed: &ConflictedContent) -> Vec<&Conflict> {
        parsed
            .pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Conflict(conflict) => Some(conflict),
                Piece::Text { .. } => None,
            })
            .collect()
    }

    fn text_pieces(parsed: &ConflictedContent) -> Vec<Vec<String>> {
        parsed
            .pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Text { lines } => Some(lines.clone()),
                Piece::Conflict(_) => None,
            })
            .collect()
    }

    /// jj's default style, and the one that needs real work: side #1 is not
    /// written out anywhere — it has to be reconstructed by applying the diff.
    #[test]
    fn a_diff_style_conflict_yields_both_sides_and_the_base() {
        let parsed = parse_conflicts(
            "before\n\
             <<<<<<< Conflict 1 of 1\n\
             %%%%%%% Changes from base to side #1\n\
             -shared\n\
             -was base\n\
             +shared\n\
             +is side one\n\
             +++++++ Contents of side #2\n\
             shared\n\
             is side two\n\
             >>>>>>> Conflict 1 of 1 ends\n\
             after\n",
        );
        let regions = conflicts(&parsed);
        assert_eq!(regions.len(), 1);
        let region = regions[0];
        assert_eq!(region.label, "Conflict 1 of 1");
        assert_eq!(region.sides.len(), 2, "{:?}", region.sides);
        assert_eq!(region.sides[0].lines, vec!["shared", "is side one"]);
        assert_eq!(region.sides[1].lines, vec!["shared", "is side two"]);
        assert_eq!(
            region.base.as_ref().unwrap().lines,
            vec!["shared", "was base"],
            "the base is what the diff was from"
        );
        assert_eq!(text_pieces(&parsed), vec![vec!["before"], vec!["after"]]);
        assert!(parsed.trailing_newline);
    }

    /// A context line inside a diff section belongs to *both* sides. Getting
    /// this wrong is invisible in a one-line conflict and mangles every real one.
    #[test]
    fn diff_context_lines_land_on_the_base_and_the_side() {
        let parsed = parse_conflicts(
            "<<<<<<< Conflict 1 of 1\n\
             %%%%%%% Changes from base to side #1\n\
             \x20keep me\n\
             -gone\n\
             +new\n\
             +++++++ Contents of side #2\n\
             other\n\
             >>>>>>> Conflict 1 of 1 ends\n",
        );
        let region = conflicts(&parsed)[0];
        assert_eq!(region.sides[0].lines, vec!["keep me", "new"]);
        assert_eq!(region.base.as_ref().unwrap().lines, vec!["keep me", "gone"]);
    }

    #[test]
    fn snapshot_style_reads_every_side_verbatim() {
        let parsed = parse_conflicts(
            "<<<<<<< Conflict 1 of 1\n\
             +++++++ Contents of side #1\n\
             one\n\
             ------- Contents of base\n\
             base\n\
             +++++++ Contents of side #2\n\
             two\n\
             >>>>>>> Conflict 1 of 1 ends\n",
        );
        let region = conflicts(&parsed)[0];
        assert_eq!(region.sides.len(), 2);
        assert_eq!(region.sides[0].lines, vec!["one"]);
        assert_eq!(region.sides[1].lines, vec!["two"]);
        assert_eq!(region.base.as_ref().unwrap().lines, vec!["base"]);
    }

    /// jj does not write these, but a merge tool run over a jj tree does, and
    /// they are still a conflict someone has to resolve.
    #[test]
    fn git_style_markers_still_parse() {
        let parsed = parse_conflicts(
            "<<<<<<< HEAD\n\
             ours\n\
             ||||||| base\n\
             was\n\
             =======\n\
             theirs\n\
             >>>>>>> branch\n",
        );
        let region = conflicts(&parsed)[0];
        assert_eq!(region.sides.len(), 2, "{:?}", region.sides);
        assert_eq!(region.sides[0].lines, vec!["ours"]);
        assert_eq!(region.sides[1].lines, vec!["theirs"]);
        assert_eq!(region.base.as_ref().unwrap().lines, vec!["was"]);
    }

    /// One file, several conflicts — the ordinary case, and the reason regions
    /// rather than files are what the UI steps between.
    #[test]
    fn several_regions_in_one_file_are_numbered_in_order() {
        let region = |n: u32| {
            format!(
                "<<<<<<< Conflict {n} of 2\n\
                 +++++++ Contents of side #1\n\
                 a{n}\n\
                 +++++++ Contents of side #2\n\
                 b{n}\n\
                 >>>>>>> Conflict {n} of 2 ends\n"
            )
        };
        let parsed = parse_conflicts(&format!("head\n{}middle\n{}tail\n", region(1), region(2)));
        let regions = conflicts(&parsed);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].index, 0);
        assert_eq!(regions[1].index, 1);
        assert_eq!(regions[1].sides[0].lines, vec!["a2"]);
        assert_eq!(parsed.conflict_count(), 2);
        assert_eq!(
            text_pieces(&parsed),
            vec![vec!["head"], vec!["middle"], vec!["tail"]],
            "text between regions stays where it was"
        );
    }

    /// jj lengthens its markers when the conflicted content would otherwise
    /// close the region. A parser fixed at seven characters mis-reads exactly
    /// the files that were careful.
    #[test]
    fn longer_markers_are_markers_and_content_that_looks_like_one_is_not() {
        let parsed = parse_conflicts(
            "<<<<<<<<< Conflict 1 of 1\n\
             +++++++++ Contents of side #1\n\
             <<<<<<< not a marker, just content\n\
             +++++++++ Contents of side #2\n\
             two\n\
             >>>>>>>>> Conflict 1 of 1 ends\n",
        );
        let regions = conflicts(&parsed);
        assert_eq!(regions.len(), 1);
        assert_eq!(
            regions[0].sides[0].lines,
            vec!["<<<<<<< not a marker, just content"],
            "the shorter fence inside a longer region is content"
        );
    }

    #[test]
    fn text_that_is_not_a_conflict_comes_back_unchanged() {
        let parsed = parse_conflicts("just\nsome\nlines\n");
        assert_eq!(parsed.conflict_count(), 0);
        assert_eq!(text_pieces(&parsed), vec![vec!["just", "some", "lines"]]);
    }

    /// A `<<<<<<<` with no `>>>>>>>` is either a truncated file or a line of
    /// content. Either way it must not eat the rest of the file.
    #[test]
    fn an_unterminated_region_stays_ordinary_text() {
        let parsed = parse_conflicts("<<<<<<< looks like one\nbody\ntail\n");
        assert_eq!(parsed.conflict_count(), 0);
        assert_eq!(text_pieces(&parsed), vec![vec!["<<<<<<< looks like one", "body", "tail"]]);
    }

    #[test]
    fn a_missing_trailing_newline_is_recorded_rather_than_added() {
        assert!(!parse_conflicts("no newline").trailing_newline);
        assert!(parse_conflicts("newline\n").trailing_newline);
    }

    #[test]
    fn resolutions_are_checked_for_leftover_fences() {
        assert!(has_conflict_markers("a\n<<<<<<< Conflict 1 of 1\nb\n"));
        assert!(has_conflict_markers("a\n>>>>>>> Conflict 1 of 1 ends\n"));
        assert!(!has_conflict_markers("a\nb\n"));
        assert!(
            !has_conflict_markers("a\n+++++++ leftover side label\n"),
            "only the fences decide: a plus-run is ordinary content in plenty of files"
        );
    }
}
