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

    let mut worktree = collect_worktree(repo_root, global_excludes(&repo).as_deref())?;
    restore_tracked(repo_root, &base, &mut worktree);

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

/// git's global ignore file (`core.excludesFile`), resolved through the repo's
/// own config so `~` and `$XDG_CONFIG_HOME` are expanded the way git expands
/// them.
///
/// This has to be resolved and handed to the walker explicitly. `ignore` will
/// find it on its own, but the matcher it builds for it is anchored to the
/// **process's current directory** rather than to the directory being walked —
/// so the rules applied only when jjdiff happened to be launched from the repo
/// root. Everywhere else (`pnpm tauri dev` runs from `src-tauri/`, a bundled
/// `.app` from `/`) globally-ignored files showed up in the diff that `jj st`
/// does not list, which reads as jjdiff and jj disagreeing about the change.
fn global_excludes(repo: &gix::Repository) -> Option<std::path::PathBuf> {
    let config = repo.config_snapshot();
    let path = config.trusted_path("core.excludesFile").ok()??.to_owned();
    path.exists().then_some(path)
}

/// The global excludes as a matcher **rooted at the repo**, which is the whole
/// point: both `WalkBuilder`'s own global handling and its `add_ignore` build
/// their matcher against the empty path, i.e. the process's current directory,
/// so neither matches a path under the repo unless the two happen to coincide.
fn global_matcher(root: &Path, excludes: Option<&Path>) -> Option<ignore::gitignore::Gitignore> {
    let excludes = excludes?;
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    if builder.add(excludes).is_some() {
        return None; // unreadable or malformed — ignore rules are best-effort
    }
    builder.build().ok()
}

/// Gitignore-aware walk → path → file size. Skips `.git`, `.jj`, and symlinks.
fn collect_worktree(
    root: &Path,
    global_excludes: Option<&Path>,
) -> Result<BTreeMap<String, u64>, DiffError> {
    let mut files = BTreeMap::new();
    let global = global_matcher(root, global_excludes);
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| {
            if entry.file_name() == ".git" || entry.file_name() == ".jj" {
                return false;
            }
            // A directory with its own `.git` is a different repository, and its
            // contents belong to that one. git collapses an untracked nested repo
            // to a single `?? dir/` entry and never lists inside it; jj does not
            // snapshot it at all. jjdiff walked straight in, so a vendored clone
            // sitting in a subdirectory turned up as dozens of untracked
            // additions that no other tool reports — 44 of them on the repo this
            // was found on, left over after the ignored-but-tracked fix.
            //
            // Depth guards the root, which has a `.git` by definition; a
            // *committed* submodule is a gitlink rather than a blob, so it never
            // reaches `base` and needs nothing here.
            if entry.depth() > 0 && entry.file_type().is_some_and(|kind| kind.is_dir()) {
                return !entry.path().join(".git").exists();
            }
            true
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
        if let Some(global) = &global {
            // `_or_any_parents` so a globally-ignored *directory* takes its
            // contents with it, the way git treats one.
            if global.matched_path_or_any_parents(entry.path(), false).is_ignore() {
                continue;
            }
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(relative) = entry.path().strip_prefix(root) else { continue };
        let path = relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        files.insert(path, metadata.len());
    }
    Ok(files)
}

/// Put back the files the ignore-aware walk was never allowed to see.
///
/// **An ignore rule applies to untracked files only.** Once a file is in the
/// tree it is tracked, and git, jj and every other tool go on diffing it however
/// many patterns match its path — `.gitignore` decides what gets *added*, not
/// what gets watched. [`collect_worktree`] cannot express that on its own: the
/// walker prunes an ignored *directory* without descending, so a tracked file
/// inside one is not filtered out so much as never visited.
///
/// The consequence was severe and looked like data loss. A repo that ignores
/// `.vscode/` while committing `.vscode/settings.json` — a common arrangement,
/// and the reason this was found — had every such file reported as **deleted**
/// against a working copy where it was sitting untouched on disk: 1467 phantom
/// deletions on the repo that turned it up, next to `jj st`'s one modified file.
///
/// So the tree is the authority on what to look at, and the walk only adds to
/// it. Costs one `lstat` per tracked path the walk missed — zero on a repo with
/// no ignored-but-tracked files, and only over the difference on one that has
/// them. A path that really is gone still fails to stat and stays a deletion,
/// and symlinks stay out, both as before.
fn restore_tracked(root: &Path, base: &BTreeMap<String, gix::ObjectId>, worktree: &mut BTreeMap<String, u64>) {
    for path in base.keys() {
        if worktree.contains_key(path) {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(root.join(path)) else { continue };
        if metadata.is_file() {
            worktree.insert(path.clone(), metadata.len());
        }
    }
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
            .args([
                "--config",
                "user.name=Test",
                "--config",
                "user.email=t@example.com",
                // Hermetic: the developer's global config may enable commit signing,
                // and a locked/absent SSH agent would fail repo creation here.
                "--config",
                "signing.behavior=drop",
            ])
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

    /// The global excludes file must be honoured, and honoured the same way
    /// wherever the process happens to be standing.
    ///
    /// This is not hypothetical: `ignore`'s own global handling anchors its
    /// matcher to the current directory, so jjdiff applied `core.excludesFile`
    /// only when launched from the repo root. `pnpm tauri dev` runs from
    /// `src-tauri/` and a bundled `.app` from `/`, so the shipped app listed
    /// files `jj st` does not — the two tools disagreeing about what is in the
    /// change. Hence the cwd loop rather than a single call.
    #[test]
    fn global_excludes_apply_from_any_working_directory() {
        if Command::new("jj").arg("--version").output().is_err() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let (tmp, base) = fixture();
        let root = tmp.path();

        // A global excludes file, declared where git declares one. It goes in
        // the *git* config, not jj's: gix is what resolves it, and a colocated
        // repo is where jjdiff finds it.
        let excludes = root.join("global-ignore");
        std::fs::write(&excludes, "secret*.txt\n").unwrap();
        let out = Command::new("git")
            .args(["config", "--local", "core.excludesFile", excludes.to_str().unwrap()])
            .current_dir(root)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        std::fs::write(root.join("secret-notes.txt"), "private\n").unwrap();

        // `diff_worktree` takes an absolute root, so the only thing varying is
        // where the *process* stands — which must not matter. cwd is
        // process-global and these tests run threaded, so it is restored
        // before any assertion can panic out from under the other tests.
        let elsewhere = std::env::temp_dir();
        let mut runs: Vec<(std::path::PathBuf, Vec<String>)> = Vec::new();
        let previous = std::env::current_dir().unwrap();
        for cwd in [root, elsewhere.as_path()] {
            std::env::set_current_dir(cwd).unwrap();
            let files = diff_worktree(root, Some(&base), WorktreeDiffOptions::default());
            runs.push((
                cwd.to_path_buf(),
                files.unwrap_or_default().into_iter().map(|file| file.path).collect(),
            ));
        }
        std::env::set_current_dir(previous).unwrap();

        for (cwd, paths) in runs {
            assert!(
                !paths.iter().any(|path| path == "secret-notes.txt"),
                "globally-excluded file leaked when run from {}: {paths:?}",
                cwd.display()
            );
            assert!(
                paths.iter().any(|path| path == "fresh.txt"),
                "unrelated files must survive: {paths:?}"
            );
        }
    }

    /// A tracked file that an ignore rule also matches is still tracked.
    ///
    /// `.gitignore` decides what gets added, not what gets watched, and every
    /// tool that diffs — git, jj — goes on reporting a committed file however
    /// many patterns cover it. jjdiff did not: the walker prunes an ignored
    /// directory without descending, so the file was never visited and came out
    /// the other side as a **deletion** of something sitting untouched on disk.
    ///
    /// Both shapes here, because they fail for different reasons: an ignored
    /// *directory* is pruned whole, an ignored *file* is filtered by name.
    /// `.vscode/` committing its `settings.json` is the arrangement that turned
    /// this up, and it produced 1467 phantom deletions on one repo.
    #[test]
    fn tracked_files_survive_an_ignore_rule_that_matches_them() {
        if Command::new("jj").arg("--version").output().is_err() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        jj(root, &["git", "init", "--colocate", "."]);

        // Neutralise the developer's own `core.excludesFile`. This test is about
        // ignore semantics, and a machine whose global ignore already covers
        // `.vscode/` or `*.log` — mine does — would never track the fixture
        // files in the first place and the assertions would pass vacuously.
        std::fs::write(root.join("empty-excludes"), "").unwrap();
        let out = Command::new("git")
            .args([
                "config",
                "--local",
                "core.excludesFile",
                root.join("empty-excludes").to_str().unwrap(),
            ])
            .current_dir(root)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

        // Committed first, ignored after — which is how a repo ends up in this
        // state, whether by a rule added later or a deliberate `git add -f`.
        std::fs::create_dir(root.join("tools")).unwrap();
        std::fs::write(root.join("tools/settings.json"), "{\"a\": 1}\n").unwrap();
        std::fs::write(root.join("keep.data"), "kept\n").unwrap();
        std::fs::write(root.join("plain.txt"), "one\n").unwrap();
        jj(root, &["commit", "-m", "before the rules"]);
        std::fs::write(root.join(".gitignore"), "tools/\nkeep.data\n").unwrap();
        jj(root, &["commit", "-m", "add the rules"]);

        let out = Command::new("jj")
            .args(["--ignore-working-copy", "--color=never", "log", "--no-graph", "-r", "@-", "-T", "commit_id"])
            .current_dir(root)
            .output()
            .unwrap();
        let base = String::from_utf8(out.stdout).unwrap().trim().to_string();

        // Nothing has been touched, so nothing may be reported.
        let clean = diff_worktree(root, Some(&base), WorktreeDiffOptions::default()).unwrap();
        assert!(
            clean.is_empty(),
            "an untouched worktree reported changes: {:?}",
            clean.iter().map(|f| (&f.path, f.status)).collect::<Vec<_>>()
        );

        // Edited, they diff like anything else — not as add/delete pairs.
        std::fs::write(root.join("tools/settings.json"), "{\"a\": 2}\n").unwrap();
        std::fs::write(root.join("keep.data"), "changed\n").unwrap();
        let edited = diff_worktree(root, Some(&base), WorktreeDiffOptions::default()).unwrap();
        let mut seen: Vec<(&str, FileStatus)> =
            edited.iter().map(|f| (f.path.as_str(), f.status)).collect();
        seen.sort_by_key(|(path, _)| *path);
        assert_eq!(
            seen,
            vec![("keep.data", FileStatus::Modified), ("tools/settings.json", FileStatus::Modified)]
        );

        // And a tracked-but-ignored file that is genuinely gone is still a
        // deletion — the fix restores what is on disk, it does not invent it.
        std::fs::remove_file(root.join("tools/settings.json")).unwrap();
        let removed = diff_worktree(root, Some(&base), WorktreeDiffOptions::default()).unwrap();
        assert_eq!(
            removed
                .iter()
                .find(|f| f.path == "tools/settings.json")
                .map(|f| f.status),
            Some(FileStatus::Deleted)
        );

        // An *untracked* file matching a rule stays ignored, which is the whole
        // point of the walk this works around.
        std::fs::write(root.join("keep.data.bak"), "noise\n").unwrap();
        std::fs::write(root.join("tools/scratch.json"), "{}\n").unwrap();
        std::fs::write(root.join("plain2.txt"), "two\n").unwrap();
        let untracked = diff_worktree(root, Some(&base), WorktreeDiffOptions::default()).unwrap();
        let paths: Vec<&str> = untracked.iter().map(|f| f.path.as_str()).collect();
        assert!(!paths.contains(&"tools/scratch.json"), "ignored file leaked: {paths:?}");
        assert!(paths.contains(&"plain2.txt"), "unignored file lost: {paths:?}");
    }

    /// A directory with its own `.git` belongs to another repository.
    ///
    /// git collapses an untracked nested repo to one `?? dir/` and never lists
    /// inside it; jj does not snapshot it at all. jjdiff walked in and reported
    /// every file as an addition — which is how a vendored clone of an unrelated
    /// project showed up as 44 phantom additions in a review.
    #[test]
    fn a_nested_git_repository_is_not_part_of_this_one() {
        if Command::new("jj").arg("--version").output().is_err() {
            eprintln!("skipping: jj not installed");
            return;
        }
        let (tmp, base) = fixture();
        let root = tmp.path();

        let nested = root.join("vendor/thing");
        std::fs::create_dir_all(&nested).unwrap();
        let out = Command::new("git")
            .args(["init", "-q", "."])
            .current_dir(&nested)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        std::fs::write(nested.join("theirs.txt"), "not ours\n").unwrap();
        // A plain subdirectory beside it must still be walked.
        std::fs::create_dir_all(root.join("vendor/ours")).unwrap();
        std::fs::write(root.join("vendor/ours/mine.txt"), "ours\n").unwrap();

        let paths: Vec<String> = diff_worktree(root, Some(&base), WorktreeDiffOptions::default())
            .unwrap()
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert!(
            !paths.iter().any(|path| path.starts_with("vendor/thing")),
            "walked into a nested repository: {paths:?}"
        );
        assert!(
            paths.iter().any(|path| path == "vendor/ours/mine.txt"),
            "an ordinary subdirectory was lost with it: {paths:?}"
        );
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
