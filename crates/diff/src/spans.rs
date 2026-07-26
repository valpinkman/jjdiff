//! Intra-line word emphasis.
//!
//! Within each hunk, runs of removed lines are paired index-wise with the following run of
//! added lines; each pair is token-diffed and the changed ranges become `Line::spans`. When a
//! line changed almost completely, spans are dropped — a fully-marked line reads worse than a
//! plain colored one.

use similar::{capture_diff_slices, Algorithm, DiffOp};

use crate::{Hunk, LineKind};

/// `[start, end)` emphasis range in UTF-16 code units.
pub type Span = (u32, u32);

/// Fraction of a line that may be emphasized before we give up on spans.
const MAX_MARKED_FRACTION: f32 = 0.7;

pub fn add_word_spans(hunk: &mut Hunk) {
    // Collect (removed-run, added-run) pair boundaries first to appease the borrow checker.
    let mut pairs: Vec<(usize, usize)> = Vec::new(); // (removed index, added index)
    let mut removed_run: Vec<usize> = Vec::new();
    let mut added_run: Vec<usize> = Vec::new();
    let flush = |removed: &mut Vec<usize>, added: &mut Vec<usize>, pairs: &mut Vec<(usize, usize)>| {
        for (r, a) in removed.iter().zip(added.iter()) {
            pairs.push((*r, *a));
        }
        removed.clear();
        added.clear();
    };

    for (index, line) in hunk.lines.iter().enumerate() {
        match line.kind {
            LineKind::Removed => {
                if !added_run.is_empty() {
                    flush(&mut removed_run, &mut added_run, &mut pairs);
                }
                removed_run.push(index);
            }
            LineKind::Added => added_run.push(index),
            LineKind::Context => flush(&mut removed_run, &mut added_run, &mut pairs),
        }
    }
    flush(&mut removed_run, &mut added_run, &mut pairs);

    for (removed_index, added_index) in pairs {
        let (old_spans, new_spans) =
            word_spans(&hunk.lines[removed_index].text, &hunk.lines[added_index].text);
        hunk.lines[removed_index].spans = old_spans;
        hunk.lines[added_index].spans = new_spans;
    }
}

/// Token: a word (`[A-Za-z0-9_]+`), a whitespace run, or a single symbol char.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut index = 0;
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80;
    let is_space = |b: u8| b == b' ' || b == b'\t';
    while index < bytes.len() {
        let byte = bytes[index];
        let class = if is_word(byte) { 0u8 } else if is_space(byte) { 1 } else { 2 };
        index += 1;
        if class == 2 {
            // Symbols are single tokens, but keep multi-byte chars whole.
            tokens.push(&text[start..index]);
            start = index;
            continue;
        }
        while index < bytes.len() {
            let next = bytes[index];
            let next_class = if is_word(next) { 0 } else if is_space(next) { 1 } else { 2 };
            if next_class != class {
                break;
            }
            index += 1;
        }
        tokens.push(&text[start..index]);
        start = index;
    }
    tokens
}

/// Changed ranges of `old` and `new`, as UTF-16 code unit `[start, end)` pairs.
pub fn word_spans(old: &str, new: &str) -> (Vec<Span>, Vec<Span>) {
    if old.is_empty() || new.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    let ops = capture_diff_slices(Algorithm::Myers, &old_tokens, &new_tokens);

    let old_offsets = utf16_offsets(&old_tokens);
    let new_offsets = utf16_offsets(&new_tokens);

    let mut old_spans = Vec::new();
    let mut new_spans = Vec::new();
    for op in &ops {
        match op {
            DiffOp::Delete { old_index, old_len, .. } => {
                push_span(&mut old_spans, &old_offsets, *old_index, *old_len);
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                push_span(&mut new_spans, &new_offsets, *new_index, *new_len);
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                push_span(&mut old_spans, &old_offsets, *old_index, *old_len);
                push_span(&mut new_spans, &new_offsets, *new_index, *new_len);
            }
            DiffOp::Equal { .. } => {}
        }
    }

    if too_marked(&old_spans, old) || too_marked(&new_spans, new) {
        return (Vec::new(), Vec::new());
    }
    (old_spans, new_spans)
}

/// Prefix table: `offsets[i]` = UTF-16 offset of token `i`; last entry = total length.
fn utf16_offsets(tokens: &[&str]) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(tokens.len() + 1);
    let mut total = 0u32;
    offsets.push(0);
    for token in tokens {
        total += token.encode_utf16().count() as u32;
        offsets.push(total);
    }
    offsets
}

fn push_span(spans: &mut Vec<Span>, offsets: &[u32], index: usize, len: usize) {
    if len == 0 {
        return;
    }
    let start = offsets[index];
    let end = offsets[index + len];
    // Merge with the previous span when adjacent or overlapping.
    if let Some(last) = spans.last_mut() {
        if last.1 >= start {
            last.1 = last.1.max(end);
            return;
        }
    }
    spans.push((start, end));
}

fn too_marked(spans: &[Span], text: &str) -> bool {
    if spans.is_empty() {
        return false;
    }
    let marked: u32 = spans.iter().map(|(s, e)| e - s).sum();
    let total = text.encode_utf16().count().max(1) as f32;
    marked as f32 / total > MAX_MARKED_FRACTION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hunk, Line, LineKind};

    #[test]
    fn small_edit_gets_tight_spans() {
        let (old, new) = word_spans("const value = compute(input);", "const value = compute(output);");
        assert_eq!(old, vec![(22, 27)]); // "input"
        assert_eq!(new, vec![(22, 28)]); // "output"
    }

    #[test]
    fn complete_rewrite_gets_no_spans() {
        let (old, new) = word_spans("alpha beta gamma", "one two three four");
        assert!(old.is_empty());
        assert!(new.is_empty());
    }

    #[test]
    fn offsets_are_utf16() {
        // "日本" is 2 UTF-16 units but 6 UTF-8 bytes; a trailing edit must account for that.
        let (old, new) = word_spans("日本 alpha", "日本 beta");
        assert_eq!(old, vec![(3, 8)]);
        assert_eq!(new, vec![(3, 7)]);
    }

    #[test]
    fn pairs_removed_and_added_runs() {
        let mut hunk = Hunk {
            id: String::new(),
            old_start: 1,
            old_lines: 2,
            new_start: 1,
            new_lines: 2,
            context: String::new(),
            lines: vec![
                Line::new(LineKind::Context, "unchanged"),
                Line::new(LineKind::Removed, "let a = 1;"),
                Line::new(LineKind::Added, "let a = 2;"),
            ],
        };
        add_word_spans(&mut hunk);
        assert_eq!(hunk.lines[1].spans, vec![(8, 9)]);
        assert_eq!(hunk.lines[2].spans, vec![(8, 9)]);
    }
}
