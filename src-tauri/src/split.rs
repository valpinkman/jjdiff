//! Hunk-level split: the plan, and the scripted diff editor that carries it out.
//!
//! `jj split -i` writes the two sides of a change into a pair of directories,
//! runs the configured diff editor, and takes whatever the *right* one holds
//! when it exits. That is the only non-interactive seam jj offers for selecting
//! part of a change, so jjdiff plays the editor: [`Repo::split_with_diff_editor`]
//! registers this binary as the tool for one invocation, and jj re-enters it as
//! `jjdiff --apply-split-plan <plan> <left> <right>`.
//!
//! [`Repo::split_with_diff_editor`]: jjdiff_vcs::Repo::split_with_diff_editor
//!
//! The plan is written by the frontend from the diff the reviewer was actually
//! looking at, which is the point: the hunks applied here are the hunks on
//! screen, not a fresh decomposition that might have cut them elsewhere.
//! [`jjdiff_diff::apply_selected_hunks`] refuses to write anything unless the
//! plan still describes the files jj laid down, so a change that moved between
//! reading the diff and confirming the split aborts the whole operation rather
//! than half-applying to the wrong offsets.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use jjdiff_diff::{apply_selected_hunks, PlanHunk};

/// What happens to one file of the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Select {
    /// The whole file's change goes into the selected half. Nothing to do — it
    /// is already what jj put in the right directory.
    All,
    /// None of it does: the right side is reverted to the left.
    None,
    /// Some of it does, hunk by hunk.
    Hunks,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitFile {
    pub path: String,
    /// The pre-rename path, for a rename. jj's diff directories key on both, so
    /// undoing a rename means restoring one and removing the other.
    #[serde(default)]
    pub old_path: Option<String>,
    pub select: Select,
    /// Every hunk of the file with its selection, present only for
    /// [`Select::Hunks`]. All of them, not just the picked ones — applying the
    /// lot has to reproduce the new side, which is the check that the plan
    /// still fits.
    #[serde(default)]
    pub hunks: Vec<PlanHunk>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitPlan {
    pub files: Vec<SplitFile>,
}

impl SplitPlan {
    /// Whether this plan actually divides the change. jj rejects a split whose
    /// selected or remaining half is empty, and its error names neither, so the
    /// question is answered here where the answer can say which end is empty.
    pub fn divides(&self) -> Result<(), String> {
        let mut selected = 0usize;
        let mut left_behind = 0usize;
        for file in &self.files {
            match file.select {
                Select::All => selected += 1,
                Select::None => left_behind += 1,
                Select::Hunks => {
                    for hunk in &file.hunks {
                        if hunk.selected {
                            selected += 1;
                        } else {
                            left_behind += 1;
                        }
                    }
                }
            }
        }
        if selected == 0 {
            return Err("nothing is selected — pick the hunks that should move into their own change".into());
        }
        if left_behind == 0 {
            return Err("everything is selected — a split needs something to leave behind".into());
        }
        Ok(())
    }
}

/// Edit `right` so it holds only the selected part of the change.
///
/// Runs inside the process jj spawned as its diff editor; the exit status is
/// what jj reads, so any error here has to abort the whole split rather than
/// leave a partly-edited directory to be committed.
pub fn apply_plan(plan_path: &Path, left: &Path, right: &Path) -> Result<(), String> {
    let raw = std::fs::read_to_string(plan_path)
        .map_err(|error| format!("cannot read the split plan at {}: {error}", plan_path.display()))?;
    let plan: SplitPlan =
        serde_json::from_str(&raw).map_err(|error| format!("malformed split plan: {error}"))?;

    for file in &plan.files {
        match file.select {
            Select::All => {}
            Select::None => revert(file, left, right)?,
            Select::Hunks => apply_hunks(file, left, right)?,
        }
    }
    Ok(())
}

/// Put the file back the way the old side had it. One rule covers every status:
/// a path the old side has is copied over, a path it does not have is removed.
/// That is a deletion undone, an addition undone, an edit undone and — with the
/// rename's two paths — a rename undone.
fn revert(file: &SplitFile, left: &Path, right: &Path) -> Result<(), String> {
    for path in [Some(&file.path), file.old_path.as_ref()].into_iter().flatten() {
        let source = join_inside(left, path)?;
        let target = join_inside(right, path)?;
        if source.exists() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("{}: {error}", parent.display()))?;
            }
            // Copying carries the mode across, which is how the executable bit
            // survives — but jj checks its side out read-only, so the write bit
            // has to be put back or a later step cannot touch the file.
            std::fs::copy(&source, &target).map_err(|error| format!("{path}: {error}"))?;
            make_writable(&target);
        } else if target.exists() {
            std::fs::remove_file(&target).map_err(|error| format!("{path}: {error}"))?;
        }
    }
    Ok(())
}

fn apply_hunks(file: &SplitFile, left: &Path, right: &Path) -> Result<(), String> {
    let source = join_inside(left, &file.path)?;
    let target = join_inside(right, &file.path)?;
    let read = |path: &PathBuf| {
        std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error} (only text files can be split by hunk)", file.path))
    };
    let content = apply_selected_hunks(&file.path, &read(&source)?, &read(&target)?, &file.hunks)
        .map_err(|error| error.to_string())?;
    std::fs::write(&target, content).map_err(|error| format!("{}: {error}", file.path))
}

/// Resolve a plan path against one of jj's directories, refusing anything that
/// could leave it. The plan is jjdiff's own, but it arrives as a file path on a
/// command line, and a path is not a place to start trusting things.
fn join_inside(base: &Path, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    let ordinary = relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    if !ordinary {
        return Err(format!("refusing a path outside the diff directory: {path}"));
    }
    Ok(base.join(relative))
}

#[cfg(unix)]
fn make_writable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o200);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

#[cfg(not(unix))]
fn make_writable(path: &Path) {
    if let Ok(metadata) = std::fs::metadata(path) {
        let mut permissions = metadata.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = std::fs::set_permissions(path, permissions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jjdiff_diff::{LineKind, PlanLine};

    fn hunk(selected: bool, old_start: u32, old_lines: u32, lines: &[(LineKind, &str)]) -> PlanHunk {
        PlanHunk {
            selected,
            old_start,
            old_lines,
            lines: lines
                .iter()
                .map(|(kind, text)| PlanLine { kind: *kind, text: (*text).to_string() })
                .collect(),
        }
    }

    /// Stand in for what jj hands a diff editor: a left dir and a right dir.
    struct Dirs {
        _tmp: tempfile::TempDir,
        left: PathBuf,
        right: PathBuf,
    }

    fn dirs(files: &[(&str, Option<&str>, Option<&str>)]) -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        std::fs::create_dir_all(&left).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        for (path, old, new) in files {
            if let Some(content) = old {
                std::fs::write(left.join(path), content).unwrap();
            }
            if let Some(content) = new {
                std::fs::write(right.join(path), content).unwrap();
            }
        }
        Dirs { _tmp: tmp, left, right }
    }

    fn run(plan: &SplitPlan, dirs: &Dirs) -> Result<(), String> {
        let path = dirs.left.parent().unwrap().join("plan.json");
        std::fs::write(&path, serde_json::to_string(plan).unwrap()).unwrap();
        apply_plan(&path, &dirs.left, &dirs.right)
    }

    #[test]
    fn a_partial_selection_rewrites_only_the_picked_hunks() {
        let dirs = dirs(&[("f.txt", Some("one\ntwo\nthree\nfour\nfive\n"), Some("ONE\ntwo\nthree\nfour\nFIVE\n"))]);
        let plan = SplitPlan {
            files: vec![SplitFile {
                path: "f.txt".into(),
                old_path: None,
                select: Select::Hunks,
                hunks: vec![
                    hunk(true, 1, 2, &[(LineKind::Removed, "one"), (LineKind::Added, "ONE"), (LineKind::Context, "two")]),
                    hunk(false, 4, 2, &[(LineKind::Context, "four"), (LineKind::Removed, "five"), (LineKind::Added, "FIVE")]),
                ],
            }],
        };
        run(&plan, &dirs).unwrap();
        assert_eq!(
            std::fs::read_to_string(dirs.right.join("f.txt")).unwrap(),
            "ONE\ntwo\nthree\nfour\nfive\n"
        );
    }

    #[test]
    fn select_all_leaves_the_right_side_exactly_as_jj_wrote_it() {
        let dirs = dirs(&[("f.txt", Some("old\n"), Some("new\n"))]);
        let plan = SplitPlan {
            files: vec![SplitFile { path: "f.txt".into(), old_path: None, select: Select::All, hunks: vec![] }],
        };
        run(&plan, &dirs).unwrap();
        assert_eq!(std::fs::read_to_string(dirs.right.join("f.txt")).unwrap(), "new\n");
    }

    /// One rule, four statuses. Each of these is the whole reason `revert`
    /// keys on "does the old side have this path" rather than on the status.
    #[test]
    fn select_none_undoes_an_edit_an_addition_and_a_deletion() {
        let dirs = dirs(&[
            ("edited.txt", Some("old\n"), Some("new\n")),
            ("added.txt", None, Some("brand new\n")),
            ("deleted.txt", Some("still here\n"), None),
        ]);
        let none = |path: &str| SplitFile {
            path: path.into(),
            old_path: None,
            select: Select::None,
            hunks: vec![],
        };
        let plan = SplitPlan {
            files: vec![none("edited.txt"), none("added.txt"), none("deleted.txt")],
        };
        run(&plan, &dirs).unwrap();
        assert_eq!(std::fs::read_to_string(dirs.right.join("edited.txt")).unwrap(), "old\n");
        assert!(!dirs.right.join("added.txt").exists(), "an unselected addition must not appear");
        assert_eq!(
            std::fs::read_to_string(dirs.right.join("deleted.txt")).unwrap(),
            "still here\n",
            "an unselected deletion must be put back"
        );
    }

    #[test]
    fn select_none_undoes_a_rename_from_both_ends() {
        let dirs = dirs(&[("before.txt", Some("body\n"), None), ("after.txt", None, Some("body\n"))]);
        let plan = SplitPlan {
            files: vec![SplitFile {
                path: "after.txt".into(),
                old_path: Some("before.txt".into()),
                select: Select::None,
                hunks: vec![],
            }],
        };
        run(&plan, &dirs).unwrap();
        assert!(dirs.right.join("before.txt").exists(), "the old name comes back");
        assert!(!dirs.right.join("after.txt").exists(), "the new one goes away");
    }

    /// jj checks its side out read-only. A restored file that stays read-only
    /// is a file the next step cannot touch.
    #[cfg(unix)]
    #[test]
    fn a_restored_file_is_writable_even_though_jjs_side_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let dirs = dirs(&[("f.txt", Some("old\n"), Some("new\n"))]);
        std::fs::set_permissions(dirs.left.join("f.txt"), std::fs::Permissions::from_mode(0o444)).unwrap();
        let plan = SplitPlan {
            files: vec![SplitFile { path: "f.txt".into(), old_path: None, select: Select::None, hunks: vec![] }],
        };
        run(&plan, &dirs).unwrap();
        let mode = std::fs::metadata(dirs.right.join("f.txt")).unwrap().permissions().mode();
        assert_eq!(mode & 0o200, 0o200, "restored file should be writable, mode {mode:o}");
    }

    /// A plan whose hunks no longer describe the file must abort the split, not
    /// patch the wrong lines. jj reads the exit status, so an `Err` here is what
    /// keeps the commit intact.
    #[test]
    fn a_stale_plan_is_refused_rather_than_applied() {
        let dirs = dirs(&[("f.txt", Some("edited since\ntwo\n"), Some("ONE\ntwo\n"))]);
        let plan = SplitPlan {
            files: vec![SplitFile {
                path: "f.txt".into(),
                old_path: None,
                select: Select::Hunks,
                hunks: vec![hunk(true, 1, 1, &[(LineKind::Removed, "one"), (LineKind::Added, "ONE")])],
            }],
        };
        let error = run(&plan, &dirs).unwrap_err();
        assert!(error.contains("no longer matches"), "{error}");
        assert_eq!(
            std::fs::read_to_string(dirs.right.join("f.txt")).unwrap(),
            "ONE\ntwo\n",
            "nothing was written"
        );
    }

    #[test]
    fn paths_cannot_escape_the_diff_directory() {
        let dirs = dirs(&[]);
        let plan = SplitPlan {
            files: vec![SplitFile {
                path: "../escape.txt".into(),
                old_path: None,
                select: Select::None,
                hunks: vec![],
            }],
        };
        let error = run(&plan, &dirs).unwrap_err();
        assert!(error.contains("outside the diff directory"), "{error}");
    }

    #[test]
    fn a_plan_that_does_not_divide_the_change_is_rejected_before_jj_runs() {
        let all = SplitPlan {
            files: vec![SplitFile { path: "f".into(), old_path: None, select: Select::All, hunks: vec![] }],
        };
        assert!(all.divides().unwrap_err().contains("everything is selected"));

        let none = SplitPlan {
            files: vec![SplitFile { path: "f".into(), old_path: None, select: Select::None, hunks: vec![] }],
        };
        assert!(none.divides().unwrap_err().contains("nothing is selected"));

        let mixed = SplitPlan {
            files: vec![SplitFile {
                path: "f".into(),
                old_path: None,
                select: Select::Hunks,
                hunks: vec![hunk(true, 1, 1, &[]), hunk(false, 3, 1, &[])],
            }],
        };
        mixed.divides().unwrap();
    }
}
