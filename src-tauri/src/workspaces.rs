//! Where jjdiff's generated workspaces go, what they are called, and which of them it is
//! allowed to delete.
//!
//! jj decides everything about a workspace except its directory and its name, and those two
//! are exactly what a GUI has to answer for the user — `jj workspace add ../build` is a fine
//! thing to type and a bad thing to click, because clicking it a second time needs a second
//! name nobody chose.
//!
//! The layout is `<configured root>/<repo dirname>/<workspace name>`, defaulting to
//! `~/.jjdiff/workspaces`. Grouping by repository is what keeps two projects that both want
//! a workspace called `build` from colliding, and what makes the whole of one repo's
//! generated trees removable as a unit.
//!
//! The one rule with teeth is [`is_generated`]. `jj workspace forget` never touches the
//! disk, so removing the files is jjdiff's own act — and it will only perform it on a
//! directory it created. A workspace the user added themselves, wherever they put it, is
//! forgotten and left exactly where it is.

use std::path::{Path, PathBuf};

/// The directory a new workspace called `name` should be created in.
pub fn generated_path(root: &Path, repo_root: &Path, name: &str) -> PathBuf {
    let repo = repo_root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    root.join(slug(&repo)).join(name)
}

/// Whether `path` is a workspace jjdiff made, and may therefore delete.
///
/// A prefix test, deliberately: the question is not "did this session create it" — jjdiff
/// keeps no record of that and a record would be wrong across reinstalls — but "is this
/// inside the directory jjdiff owns". Anything else is the user's, including a workspace
/// they made by hand that happens to sit next to ours.
pub fn is_generated(root: Option<&Path>, path: &Path) -> bool {
    root.is_some_and(|root| path.starts_with(root) && path != root)
}

/// A workspace name derived from a change's description, unique against `taken`.
///
/// jj accepts almost anything as a name; a path has opinions, and the name becomes the last
/// component of one. So: the first line of the description, lowercased, non-word runs
/// collapsed to hyphens, truncated — the same shape a branch name would take, because it is
/// read in the same places and by the same person.
pub fn suggest_name(description: &str, taken: &[String]) -> String {
    let base = slug(description.lines().next().unwrap_or(""));
    let base = if base.is_empty() { "workspace".to_string() } else { base };
    if !taken.iter().any(|name| name == &base) {
        return base;
    }
    // `-2` rather than `-1`: the unsuffixed name is the first, so the next one a person
    // would write down is the second.
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.iter().any(|name| name == candidate))
        .expect("an unused suffix exists")
}

/// Lowercase, hyphen-separated, trimmed to something that reads as one path component.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if character == '-' || character == '_' || character == '.' {
            // Kept rather than collapsed: these already read as separators, and a repo
            // called `jj-diff.rs` should not come back as `jj-diff-rs`.
            out.push(character);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= MAX_NAME {
            break;
        }
    }
    // A leading `.` would make a hidden directory out of a visible name, and a leading or
    // trailing separator is just noise.
    out.trim_matches(['-', '.', '_'].as_slice()).to_string()
}

/// Long enough to stay recognisable, short enough to live in a path.
const MAX_NAME: usize = 40;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_derived_from_the_first_line_of_the_description() {
        assert_eq!(suggest_name("Add retry logic\n\nDetails here.", &[]), "add-retry-logic");
        assert_eq!(suggest_name("Fix: the parser (again)!", &[]), "fix-the-parser-again");
    }

    #[test]
    fn an_undescribed_change_still_gets_a_name() {
        assert_eq!(suggest_name("", &[]), "workspace");
        assert_eq!(suggest_name("!!!", &[]), "workspace");
    }

    #[test]
    fn collisions_are_suffixed_from_two() {
        let taken = vec!["build".to_string(), "build-2".to_string()];
        assert_eq!(suggest_name("build", &taken), "build-3");
    }

    #[test]
    fn a_name_cannot_become_a_hidden_directory_or_escape_one() {
        assert_eq!(suggest_name("../../etc/passwd", &[]), "etc-passwd");
        assert_eq!(suggest_name(".hidden", &[]), "hidden");
        assert!(!suggest_name("../..", &[]).contains('/'));
    }

    #[test]
    fn names_are_bounded() {
        let long = "a".repeat(200);
        assert!(suggest_name(&long, &[]).len() <= MAX_NAME);
    }

    #[test]
    fn the_path_groups_by_repository() {
        let path = generated_path(Path::new("/home/x/.jjdiff/workspaces"), Path::new("/p/codiff"), "build");
        assert_eq!(path, Path::new("/home/x/.jjdiff/workspaces/codiff/build"));
    }

    /// The rule that decides whether jjdiff may remove files. A workspace the
    /// user made is forgotten and left alone, wherever it is.
    #[test]
    fn only_workspaces_under_the_configured_root_are_ours_to_delete() {
        let root = PathBuf::from("/home/x/.jjdiff/workspaces");
        assert!(is_generated(Some(&root), Path::new("/home/x/.jjdiff/workspaces/codiff/build")));
        assert!(!is_generated(Some(&root), Path::new("/p/codiff-build")));
        assert!(!is_generated(Some(&root), &root), "the root itself is not a workspace");
        assert!(
            !is_generated(None, Path::new("/anywhere")),
            "with no configured root, nothing is ours"
        );
    }
}
