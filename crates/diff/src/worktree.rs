//! Live working-copy diffing without jj.
//!
//! Diffs the filesystem against a base tree (normally `@-`) by reading the colocated `.git`
//! object store with gix. This is what makes the working-copy view *live* while never taking
//! jj's working-copy lock and never writing an operation (PLAN.md).
//!
//! Known limits, all marked in output rather than silently wrong:
//! - Files inside jj conflict trees (`.jjconflict-*` entries) are reported as skipped.
//! - Symlinks and exec-bit-only changes are ignored.
//! - Rename detection is exact-content only (see [`detect_renames`]).

use std::collections::BTreeMap;
use std::path::Path;

use similar::{capture_diff_slices, group_diff_ops, Algorithm, DiffOp};

use crate::{spans, DiffError, FilePatch, FileStatus, Hunk, Line, LineKind};

/// Files larger than this are reported but not diffed.
pub const MAX_FILE_SIZE: u64 = 4 * 1024 * 1024;
const CONTEXT_LINES: usize = 3;
const BINARY_SNIFF: usize = 8192;

#[derive(Debug, Clone, Copy, Default)]
pub struct WorktreeDiffOptions {
    pub ignore_whitespace: bool,
}

/// Diff the working tree at `repo_root` against `base_commit` (git hex; `None` = empty tree).
pub fn diff_worktree(
    repo_root: &Path,
    base_commit: Option<&str>,
    options: WorktreeDiffOptions,
) -> Result<Vec<FilePatch>, DiffError> {
    let gix_err = |error: &dyn std::fmt::Display| DiffError::Gix(error.to_string());

    let repo = gix::open(repo_root).map_err(|e| gix_err(&e))?;
    let mut base = BTreeMap::new();
    let mut conflicted: Vec<String> = Vec::new();
    if let Some(hex) = base_commit {
        let oid = gix::ObjectId::from_hex(hex.as_bytes()).map_err(|e| gix_err(&e))?;
        let commit = repo
            .find_object(oid)
            .map_err(|e| gix_err(&e))?
            .try_into_commit()
            .map_err(|e| gix_err(&e))?;
        let tree = commit.tree().map_err(|e| gix_err(&e))?;
        collect_tree(&tree, &mut base, &mut conflicted).map_err(|e| gix_err(&e))?;
    }

    let worktree = collect_worktree(repo_root)?;

    let mut patches = Vec::new();

    // Conflicted paths first: visible, never content-diffed.
    for path in &conflicted {
        patches.push(skipped_patch(
            path.clone(),
            FileStatus::Modified,
            "conflicted in base revision — resolve with `jj resolve`",
        ));
    }

    let mut paths: Vec<&String> = base.keys().chain(worktree.keys()).collect();
    paths.sort();
    paths.dedup();

    for path in paths {
        let entry = base.get(path);
        let on_disk = worktree.get(path);
        let patch = match (entry, on_disk) {
            (Some(oid), None) => diff_pair(&repo, path, Some(*oid), None, options)?,
            (None, Some(size)) => diff_pair(&repo, path, None, Some((repo_root, *size)), options)?,
            (Some(oid), Some(size)) => {
                if unchanged(&repo, *oid, repo_root, path, *size)? {
                    continue;
                }
                diff_pair(&repo, path, Some(*oid), Some((repo_root, *size)), options)?
            }
            (None, None) => unreachable!(),
        };
        if let Some(patch) = patch {
            patches.push(patch);
        }
    }
    detect_renames(&mut patches);
    crate::assign_hunk_ids(&mut patches);
    Ok(patches)
}

/// Pair deletions with additions of identical content into renames.
///
/// Exact-content only: a delete and an add whose hunks carry the same lines are the same
/// file moved. Similarity-based detection (git's `-M50%`) is deliberately not attempted —
/// a wrong pairing reads far worse in review than an honest add + delete, and jj itself
/// only resolves copies at snapshot time.
fn detect_renames(patches: &mut Vec<FilePatch>) {
    let content = |patch: &FilePatch| -> Option<String> {
        if patch.binary || patch.skipped.is_some() || patch.hunks.is_empty() {
            return None;
        }
        Some(
            patch
                .hunks
                .iter()
                .flat_map(|hunk| hunk.lines.iter())
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )
    };

    let deleted: Vec<(usize, String)> = patches
        .iter()
        .enumerate()
        .filter(|(_, patch)| patch.status == FileStatus::Deleted)
        .filter_map(|(index, patch)| content(patch).map(|text| (index, text)))
        .collect();
    if deleted.is_empty() {
        return;
    }

    let mut drop_indices: Vec<usize> = Vec::new();
    for index in 0..patches.len() {
        if patches[index].status != FileStatus::Added {
            continue;
        }
        let Some(added_text) = content(&patches[index]) else { continue };
        let matched = deleted
            .iter()
            .find(|(other, text)| *text == added_text && !drop_indices.contains(other));
        if let Some((source, _)) = matched {
            drop_indices.push(*source);
            patches[index].status = FileStatus::Renamed;
            patches[index].old_path = Some(patches[*source].path.clone());
            // A pure rename has no content delta to review.
            patches[index].hunks.clear();
            patches[index].added = 0;
            patches[index].removed = 0;
        }
    }

    drop_indices.sort_unstable();
    for index in drop_indices.into_iter().rev() {
        patches.remove(index);
    }
}

/// Walk `tree` recording blob entries; `.jjconflict-*` subtrees become conflicted paths.
fn collect_tree(
    tree: &gix::Tree<'_>,
    out: &mut BTreeMap<String, gix::ObjectId>,
    conflicted: &mut Vec<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    use gix::traverse::tree::Recorder;
    let mut recorder = Recorder::default();
    tree.traverse().breadthfirst(&mut recorder)?;
    for entry in recorder.records {
        let path = entry.filepath.to_string();
        if let Some(position) = path.find(".jjconflict") {
            // Record the real path (everything before the conflict marker component).
            let real = path[..position].trim_end_matches('/');
            if !real.is_empty() && !conflicted.iter().any(|c| c == real) {
                conflicted.push(real.to_string());
            }
            continue;
        }
        if entry.mode.is_blob() {
            out.insert(path, entry.oid);
        }
    }
    Ok(())
}

/// Gitignore-aware walk → path → file size. Skips `.git`, `.jj`, and symlinks.
fn collect_worktree(root: &Path) -> Result<BTreeMap<String, u64>, DiffError> {
    let mut files = BTreeMap::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| {
            entry.file_name() != ".git" && entry.file_name() != ".jj"
        })
        .build();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue, // permission errors etc. — skip, don't fail the whole diff
        };
        let Some(file_type) = entry.file_type() else { continue };
        if !file_type.is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(relative) = entry.path().strip_prefix(root) else { continue };
        let path = relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        files.insert(path, metadata.len());
    }
    Ok(files)
}

/// Cheap unchanged check: size mismatch → changed; equal sizes → hash the file and compare oids.
fn unchanged(
    repo: &gix::Repository,
    oid: gix::ObjectId,
    root: &Path,
    path: &str,
    fs_size: u64,
) -> Result<bool, DiffError> {
    let header = repo
        .find_header(oid)
        .map_err(|e| DiffError::Gix(e.to_string()))?;
    if header.size() != fs_size {
        return Ok(false);
    }
    let data = std::fs::read(root.join(path))?;
    let fs_oid = gix::objs::compute_hash(repo.object_hash(), gix::objs::Kind::Blob, &data)
        .map_err(|e| DiffError::Gix(e.to_string()))?;
    Ok(fs_oid == oid)
}

fn skipped_patch(path: String, status: FileStatus, reason: &str) -> FilePatch {
    FilePatch {
        path,
        old_path: None,
        status,
        binary: false,
        skipped: Some(reason.to_string()),
        added: 0,
        removed: 0,
        hunks: Vec::new(),
    }
}

/// Diff one path. `old` = base blob oid, `new` = (root, size) on disk.
fn diff_pair(
    repo: &gix::Repository,
    path: &str,
    old: Option<gix::ObjectId>,
    new: Option<(&Path, u64)>,
    options: WorktreeDiffOptions,
) -> Result<Option<FilePatch>, DiffError> {
    let status = match (old.is_some(), new.is_some()) {
        (true, false) => FileStatus::Deleted,
        (false, true) => FileStatus::Added,
        _ => FileStatus::Modified,
    };

    if let Some((_, size)) = new {
        if size > MAX_FILE_SIZE {
            return Ok(Some(skipped_patch(path.to_string(), status, "file too large")));
        }
    }

    let old_bytes = match old {
        Some(oid) => repo
            .find_object(oid)
            .map_err(|e| DiffError::Gix(e.to_string()))?
            .detach()
            .data,
        None => Vec::new(),
    };
    if old_bytes.len() as u64 > MAX_FILE_SIZE {
        return Ok(Some(skipped_patch(path.to_string(), status, "file too large")));
    }
    let new_bytes = match new {
        Some((root, _)) => std::fs::read(root.join(path))?,
        None => Vec::new(),
    };

    if is_binary(&old_bytes) || is_binary(&new_bytes) {
        let mut patch = skipped_patch(path.to_string(), status, "binary file");
        patch.binary = true;
        patch.skipped = None;
        return Ok(Some(patch));
    }

    let old_text = String::from_utf8_lossy(&old_bytes);
    let new_text = String::from_utf8_lossy(&new_bytes);
    let hunks = diff_text(&old_text, &new_text, options);
    if hunks.is_empty() && status == FileStatus::Modified {
        // Whitespace-only change under ignore_whitespace, or lossy-identical.
        return Ok(None);
    }

    let mut patch = FilePatch {
        path: path.to_string(),
        old_path: None,
        status,
        binary: false,
        skipped: None,
        added: 0,
        removed: 0,
        hunks,
    };
    patch.recount();
    Ok(Some(patch))
}

fn is_binary(bytes: &[u8]) -> bool {
    bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0)
}

/// Line-diff two texts into hunks with `CONTEXT_LINES` of context.
fn diff_text(old_text: &str, new_text: &str, options: WorktreeDiffOptions) -> Vec<Hunk> {
    let old_lines: Vec<&str> = split_lines(old_text);
    let new_lines: Vec<&str> = split_lines(new_text);

    let ops = if options.ignore_whitespace {
        let old_keys: Vec<String> = old_lines.iter().map(|l| strip_ws(l)).collect();
        let new_keys: Vec<String> = new_lines.iter().map(|l| strip_ws(l)).collect();
        capture_diff_slices(Algorithm::Myers, &old_keys, &new_keys)
    } else {
        capture_diff_slices(Algorithm::Myers, &old_lines, &new_lines)
    };

    let mut hunks = Vec::new();
    for group in group_diff_ops(ops, CONTEXT_LINES) {
        let Some(first) = group.first() else { continue };
        let Some(last) = group.last() else { continue };
        let old_start = first.old_range().start;
        let new_start = first.new_range().start;
        let old_end = last.old_range().end;
        let new_end = last.new_range().end;

        let mut hunk = Hunk {
            id: String::new(),
            old_start: old_start as u32 + 1,
            old_lines: (old_end - old_start) as u32,
            new_start: new_start as u32 + 1,
            new_lines: (new_end - new_start) as u32,
            context: String::new(),
            lines: Vec::new(),
        };

        for op in &group {
            match op {
                DiffOp::Equal { old_index, new_index, len } => {
                    for offset in 0..*len {
                        let mut line = Line::new(LineKind::Context, old_lines[old_index + offset]);
                        line.old_line = Some((old_index + offset) as u32 + 1);
                        line.new_line = Some((new_index + offset) as u32 + 1);
                        hunk.lines.push(line);
                    }
                }
                DiffOp::Delete { old_index, old_len, .. } => {
                    push_removed(&mut hunk, &old_lines, *old_index, *old_len);
                }
                DiffOp::Insert { new_index, new_len, .. } => {
                    push_added(&mut hunk, &new_lines, *new_index, *new_len);
                }
                DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                    push_removed(&mut hunk, &old_lines, *old_index, *old_len);
                    push_added(&mut hunk, &new_lines, *new_index, *new_len);
                }
            }
        }
        spans::add_word_spans(&mut hunk);
        hunks.push(hunk);
    }
    hunks
}

fn push_removed(hunk: &mut Hunk, lines: &[&str], index: usize, len: usize) {
    for offset in 0..len {
        let mut line = Line::new(LineKind::Removed, lines[index + offset]);
        line.old_line = Some((index + offset) as u32 + 1);
        hunk.lines.push(line);
    }
}

fn push_added(hunk: &mut Hunk, lines: &[&str], index: usize, len: usize) {
    for offset in 0..len {
        let mut line = Line::new(LineKind::Added, lines[index + offset]);
        line.new_line = Some((index + offset) as u32 + 1);
        hunk.lines.push(line);
    }
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    // A trailing newline produces one phantom empty element; drop it.
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

fn strip_ws(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn jj(dir: &Path, args: &[&str]) {
        let out = Command::new("jj")
            .args(["--config", "user.name=Test", "--config", "user.email=t@example.com"])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("jj runs");
        assert!(out.status.success(), "jj {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Build a colocated repo with a base commit, mutate the working tree, and return the
    /// base commit's git id.
    fn fixture() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        jj(root, &["git", "init", "--colocate", "."]);
        std::fs::write(root.join("keep.txt"), "same\n").unwrap();
        std::fs::write(root.join("edit.txt"), "alpha\nbeta\ngamma\n").unwrap();
        std::fs::write(root.join("gone.txt"), "bye\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        jj(root, &["commit", "-m", "base"]);

        // Mutations, invisible to jj until something snapshots — which we never do.
        std::fs::write(root.join("edit.txt"), "alpha\nBETA!\ngamma\n").unwrap();
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        std::fs::write(root.join("fresh.txt"), "hello\n").unwrap();
        std::fs::write(root.join("ignored.txt"), "not tracked\n").unwrap();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();

        // Base = @- = the "base" commit; read its git id without touching the working copy.
        let out = Command::new("jj")
            .args(["--ignore-working-copy", "--color=never", "log", "--no-graph", "-r", "@-", "-T", "commit_id"])
            .current_dir(root)
            .output()
            .unwrap();
        let base = String::from_utf8(out.stdout).unwrap().trim().to_string();
        (tmp, base)
    }

    #[test]
    fn diffs_live_worktree_without_snapshotting() {
        if Command::new("jj").arg("--version").output().is_err() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let (tmp, base) = fixture();
        let root = tmp.path();

        let patches =
            diff_worktree(root, Some(&base), WorktreeDiffOptions::default()).unwrap();
        let by_path: BTreeMap<&str, &FilePatch> =
            patches.iter().map(|p| (p.path.as_str(), p)).collect();

        // Changed, added, deleted, binary — and *not* keep.txt or ignored.txt.
        assert!(by_path.contains_key("edit.txt"));
        assert!(by_path.contains_key("fresh.txt"));
        assert!(by_path.contains_key("gone.txt"));
        assert!(by_path.contains_key("blob.bin"));
        assert!(!by_path.contains_key("keep.txt"));
        assert!(!by_path.contains_key("ignored.txt"));

        let edit = by_path["edit.txt"];
        assert_eq!(edit.status, FileStatus::Modified);
        assert_eq!((edit.added, edit.removed), (1, 1));
        let hunk = &edit.hunks[0];
        let removed = hunk.lines.iter().find(|l| l.kind == LineKind::Removed).unwrap();
        assert_eq!(removed.text, "beta");
        assert_eq!(removed.old_line, Some(2));

        assert_eq!(by_path["fresh.txt"].status, FileStatus::Added);
        assert_eq!(by_path["gone.txt"].status, FileStatus::Deleted);
        assert!(by_path["blob.bin"].binary);

        // The whole point: no operation was created. Op log unchanged means top op is still
        // the commit we made in the fixture.
        let out = Command::new("jj")
            .args(["--ignore-working-copy", "op", "log", "--no-graph", "-n", "1", "-T", "description"])
            .current_dir(root)
            .output()
            .unwrap();
        let top = String::from_utf8_lossy(&out.stdout);
        assert!(top.contains("commit"), "unexpected top operation: {top}");
    }

    #[test]
    fn exact_content_moves_become_renames() {
        let mut patches = vec![
            FilePatch {
                path: "old/name.rs".into(),
                old_path: None,
                status: FileStatus::Deleted,
                binary: false,
                skipped: None,
                added: 0,
                removed: 2,
                hunks: vec![Hunk {
                    id: String::new(),
                    old_start: 1,
                    old_lines: 2,
                    new_start: 0,
                    new_lines: 0,
                    context: String::new(),
                    lines: vec![
                        Line::new(LineKind::Removed, "fn main() {}"),
                        Line::new(LineKind::Removed, "// tail"),
                    ],
                }],
            },
            FilePatch {
                path: "new/name.rs".into(),
                old_path: None,
                status: FileStatus::Added,
                binary: false,
                skipped: None,
                added: 2,
                removed: 0,
                hunks: vec![Hunk {
                    id: String::new(),
                    old_start: 0,
                    old_lines: 0,
                    new_start: 1,
                    new_lines: 2,
                    context: String::new(),
                    lines: vec![
                        Line::new(LineKind::Added, "fn main() {}"),
                        Line::new(LineKind::Added, "// tail"),
                    ],
                }],
            },
        ];
        detect_renames(&mut patches);
        assert_eq!(patches.len(), 1, "the delete is consumed by the rename");
        assert_eq!(patches[0].status, FileStatus::Renamed);
        assert_eq!(patches[0].path, "new/name.rs");
        assert_eq!(patches[0].old_path.as_deref(), Some("old/name.rs"));
        assert!(patches[0].hunks.is_empty(), "a pure rename has no delta");
    }

    #[test]
    fn differing_content_stays_add_plus_delete() {
        let line = |kind, text: &str| Line::new(kind, text);
        let mut patches = vec![
            FilePatch {
                path: "a.rs".into(),
                old_path: None,
                status: FileStatus::Deleted,
                binary: false,
                skipped: None,
                added: 0,
                removed: 1,
                hunks: vec![Hunk {
                    id: String::new(),
                    old_start: 1,
                    old_lines: 1,
                    new_start: 0,
                    new_lines: 0,
                    context: String::new(),
                    lines: vec![line(LineKind::Removed, "one")],
                }],
            },
            FilePatch {
                path: "b.rs".into(),
                old_path: None,
                status: FileStatus::Added,
                binary: false,
                skipped: None,
                added: 1,
                removed: 0,
                hunks: vec![Hunk {
                    id: String::new(),
                    old_start: 0,
                    old_lines: 0,
                    new_start: 1,
                    new_lines: 1,
                    context: String::new(),
                    lines: vec![line(LineKind::Added, "two")],
                }],
            },
        ];
        detect_renames(&mut patches);
        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].status, FileStatus::Deleted);
        assert_eq!(patches[1].status, FileStatus::Added);
    }

    #[test]
    fn whitespace_mode_hides_ws_only_changes() {
        let hunks = diff_text("a\nb  c\n", "a\nb c\n", WorktreeDiffOptions { ignore_whitespace: true });
        assert!(hunks.is_empty());
        let hunks = diff_text("a\nb  c\n", "a\nb c\n", WorktreeDiffOptions::default());
        assert_eq!(hunks.len(), 1);
    }

    #[test]
    fn trailing_newline_handling() {
        assert_eq!(split_lines("a\nb\n"), vec!["a", "b"]);
        assert_eq!(split_lines("a\nb"), vec!["a", "b"]);
        assert_eq!(split_lines(""), Vec::<&str>::new());
        // M1 simplification: a trailing-newline-only change is treated as no change
        // (git renders these as "\ No newline" markers; we do not surface them yet).
        let hunks = diff_text("a", "a\n", WorktreeDiffOptions::default());
        assert!(hunks.is_empty(), "same logical lines: {hunks:?}");
    }

    #[test]
    fn empty_base_diffs_everything_as_added() {
        let hunks = diff_text("", "x\ny\n", WorktreeDiffOptions::default());
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].lines.len(), 2);
        assert!(hunks[0].lines.iter().all(|l| l.kind == LineKind::Added));
    }
}
