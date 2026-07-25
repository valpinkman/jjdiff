use crate::{DiffError, FilePatch, FileStatus, Hunk, Line, LineKind};

/// Parse `git diff` / `jj diff --git` output into [`FilePatch`]es.
pub fn parse_git_patch(patch: &str) -> Result<Vec<FilePatch>, DiffError> {
    let mut files = Vec::new();
    let mut current: Option<PendingFile> = None;
    let mut hunk: Option<Hunk> = None;

    for (index, raw) in patch.lines().enumerate() {
        if raw.starts_with("diff --git ") {
            flush(&mut files, &mut current, &mut hunk);
            current = Some(PendingFile::default());
            continue;
        }
        let Some(file) = current.as_mut() else { continue };

        if let Some(header) = raw.strip_prefix("@@ ") {
            if let Some(done) = hunk.take() {
                file.hunks.push(done);
            }
            hunk = Some(parse_hunk_header(header, index)?);
            continue;
        }

        if let Some(active) = hunk.as_mut() {
            // Inside a hunk every line is +/-/space/backslash until the next header.
            let kind = match raw.bytes().next() {
                Some(b'+') => Some(LineKind::Added),
                Some(b'-') => Some(LineKind::Removed),
                Some(b' ') | None => Some(LineKind::Context),
                Some(b'\\') => None, // "\ No newline at end of file"
                _ => None,
            };
            if let Some(kind) = kind {
                let text = if raw.is_empty() { "" } else { &raw[1..] };
                active.lines.push(Line { kind, text: text.to_string() });
                continue;
            }
            if raw.starts_with('\\') {
                continue;
            }
            // Anything else ends the hunk (start of the next file's metadata).
            file.hunks.push(hunk.take().unwrap());
        }

        if raw.starts_with("new file mode") {
            file.status = Some(FileStatus::Added);
        } else if raw.starts_with("deleted file mode") {
            file.status = Some(FileStatus::Deleted);
        } else if let Some(path) = raw.strip_prefix("rename from ") {
            file.status = Some(FileStatus::Renamed);
            file.old_path = Some(unquote(path));
        } else if let Some(path) = raw.strip_prefix("rename to ") {
            file.path = Some(unquote(path));
        } else if let Some(path) = raw.strip_prefix("--- ") {
            if path != "/dev/null" {
                file.old_path.get_or_insert_with(|| unquote(strip_ab(path)));
            }
        } else if let Some(path) = raw.strip_prefix("+++ ") {
            if path != "/dev/null" {
                file.path.get_or_insert_with(|| unquote(strip_ab(path)));
            }
        } else if raw.starts_with("Binary files ") || raw.starts_with("GIT binary patch") {
            file.binary = true;
        }
    }
    flush(&mut files, &mut current, &mut hunk);
    Ok(files)
}

#[derive(Default)]
struct PendingFile {
    path: Option<String>,
    old_path: Option<String>,
    status: Option<FileStatus>,
    binary: bool,
    hunks: Vec<Hunk>,
}

fn flush(files: &mut Vec<FilePatch>, current: &mut Option<PendingFile>, hunk: &mut Option<Hunk>) {
    let Some(mut file) = current.take() else { return };
    if let Some(done) = hunk.take() {
        file.hunks.push(done);
    }
    let status = file.status.unwrap_or(FileStatus::Modified);
    // Deletions only carry an old path; everything else prefers the new path.
    let path = match (&file.path, &file.old_path) {
        (Some(new), _) => new.clone(),
        (None, Some(old)) => old.clone(),
        (None, None) => return, // header-only garbage; drop it
    };
    let old_path = match status {
        FileStatus::Renamed => file.old_path.clone(),
        _ => None,
    };
    files.push(FilePatch { path, old_path, status, binary: file.binary, hunks: file.hunks });
}

fn parse_hunk_header(header: &str, line: usize) -> Result<Hunk, DiffError> {
    // "-l,c +l,c @@ context"
    let malformed = |message: &str| DiffError::Malformed { line: line + 1, message: message.into() };
    let end = header.find(" @@").ok_or_else(|| malformed("missing closing @@"))?;
    let (ranges, rest) = header.split_at(end);
    let context = rest.trim_start_matches(" @@").trim_start().to_string();

    let mut parts = ranges.split(' ');
    let old = parts.next().ok_or_else(|| malformed("missing old range"))?;
    let new = parts.next().ok_or_else(|| malformed("missing new range"))?;
    let (old_start, old_lines) = parse_range(old.strip_prefix('-').unwrap_or(old));
    let (new_start, new_lines) = parse_range(new.strip_prefix('+').unwrap_or(new));
    Ok(Hunk { old_start, old_lines, new_start, new_lines, context, lines: Vec::new() })
}

fn parse_range(range: &str) -> (u32, u32) {
    match range.split_once(',') {
        Some((start, lines)) => (start.parse().unwrap_or(0), lines.parse().unwrap_or(0)),
        None => (range.parse().unwrap_or(0), 1),
    }
}

fn strip_ab(path: &str) -> &str {
    path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")).unwrap_or(path)
}

/// Undo git's C-style quoting for paths with unusual characters.
fn unquote(path: &str) -> String {
    let inner = match path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
        Some(inner) => inner,
        None => return path.to_string(),
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut bytes = inner.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        match bytes.next() {
            Some(b'n') => out.push(b'\n'),
            Some(b't') => out.push(b'\t'),
            Some(b'"') => out.push(b'"'),
            Some(b'\\') => out.push(b'\\'),
            Some(digit @ b'0'..=b'7') => {
                // Up to three octal digits.
                let mut value = (digit - b'0') as u32;
                let mut clone = bytes.clone();
                for _ in 0..2 {
                    match clone.next() {
                        Some(next @ b'0'..=b'7') => {
                            value = value * 8 + (next - b'0') as u32;
                            bytes.next();
                        }
                        _ => break,
                    }
                }
                out.push(value as u8);
            }
            Some(other) => out.push(other),
            None => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from `jj diff --git` (jj 0.43.0): add + delete + rename-with-spaces + edit.
    const FIXTURE: &str = "diff --git a/added.txt b/added.txt\nnew file mode 100644\nindex 0000000000..3e757656cf\n--- /dev/null\n+++ b/added.txt\n@@ -0,0 +1,1 @@\n+new\ndiff --git a/del.txt b/del.txt\ndeleted file mode 100644\nindex 587be6b4c3..0000000000\n--- a/del.txt\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-x\ndiff --git a/a file.txt b/renamed file.txt\nrename from a file.txt\nrename to renamed file.txt\nindex f384549cbe..6addb9b7c7 100644\n--- a/a file.txt\n+++ b/renamed file.txt\n@@ -1,4 +1,4 @@\n one\n-two\n+TWO\n three\n four\n";

    #[test]
    fn parses_jj_git_output() {
        let files = parse_git_patch(FIXTURE).unwrap();
        assert_eq!(files.len(), 3);

        assert_eq!(files[0].path, "added.txt");
        assert_eq!(files[0].status, FileStatus::Added);
        assert_eq!(files[0].hunks[0].lines[0].kind, LineKind::Added);

        assert_eq!(files[1].path, "del.txt");
        assert_eq!(files[1].status, FileStatus::Deleted);

        assert_eq!(files[2].path, "renamed file.txt");
        assert_eq!(files[2].old_path.as_deref(), Some("a file.txt"));
        assert_eq!(files[2].status, FileStatus::Renamed);
        let hunk = &files[2].hunks[0];
        assert_eq!((hunk.old_start, hunk.new_start), (1, 1));
        assert_eq!(hunk.lines.len(), 5);
        assert_eq!(hunk.lines[1].kind, LineKind::Removed);
        assert_eq!(hunk.lines[2].text, "TWO");
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(parse_git_patch("").unwrap().is_empty());
    }

    #[test]
    fn unquotes_paths() {
        assert_eq!(unquote(r#""sp\303\244ce.txt""#), "späce.txt");
        assert_eq!(unquote("plain.txt"), "plain.txt");
        assert_eq!(unquote(r#""tab\there""#), "tab\there");
    }
}
