//! Per-change review state, keyed `(repo root, change id)`.
//!
//! Keying on **change ids** is the point (PLAN.md thesis #3): state survives `describe`,
//! `squash`, and rebases, and after `jj new` it stays with the change you reviewed.
//! Two maps:
//! - viewed paths (collapse in the diff)
//! - last-reviewed commit id — the anchor for "what changed since I last reviewed"
//!   interdiffs (a change id whose current commit differs has evolved since review).
//!
//! JSON-file persistence is deliberate for this scale; a database arrives with review
//! comments (M3+).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct Data {
    /// `repo\u{1}change_id` → viewed paths.
    #[serde(default)]
    viewed: HashMap<String, HashSet<String>>,
    /// `repo\u{1}change_id` → commit id at the time the change was marked reviewed.
    #[serde(default)]
    reviewed: HashMap<String, String>,
    /// `repo\u{1}change_id` → generated walkthrough (carries its own diff fingerprint).
    #[serde(default)]
    walkthroughs: HashMap<String, crate::walkthrough::Walkthrough>,
}

#[derive(Default)]
pub struct ReviewStore {
    path: Option<PathBuf>,
    data: Data,
}

impl ReviewStore {
    /// Three cases, not one. No file yet is the ordinary first run. A file we
    /// cannot read or cannot parse is a *loss*: the store stays writable, so the
    /// next toggle would write an empty map over walkthroughs that cost minutes
    /// of agent time each. Rename it aside first, timestamped so a second
    /// failure does not clobber the first rescue.
    pub fn load(path: PathBuf) -> ReviewStore {
        let data = match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str(&raw) {
                Ok(data) => data,
                Err(error) => Self::rescue(&path, error),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Data::default(),
            Err(error) => Self::rescue(&path, error),
        };
        ReviewStore { path: Some(path), data }
    }

    fn rescue(path: &std::path::Path, error: impl std::fmt::Display) -> Data {
        eprintln!("jjdiff: ignoring unreadable {}: {error}", path.display());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0);
        let aside = path.with_extension(format!("bad.{stamp}"));
        if let Err(error) = std::fs::rename(path, &aside) {
            eprintln!("jjdiff: could not set aside {}: {error}", path.display());
        } else {
            eprintln!("jjdiff: kept the old review state at {}", aside.display());
        }
        Data::default()
    }

    /// `(repo, change id)`, and **a divergent change's two commits share one entry**.
    ///
    /// Asked deliberately rather than inherited, because divergence is the one state where
    /// "the same change" is two different diffs, and the obvious repair — key the commit
    /// instead — is the wrong one. This key exists so review survives rewriting, and a
    /// divergent change is two rewrites of one change: keying the commit would discard
    /// every note on the next `describe`, imposing the failure the key was built to prevent
    /// on every repo in order to serve a rare one.
    ///
    /// It also survives the way divergence *ends*. You clear it by abandoning a side, and
    /// the notes were written about the change, not about the side — shared, they stay with
    /// whichever commit remains. Keyed per commit they would be attached to the abandoned
    /// one half the time and simply disappear.
    ///
    /// What crossing over costs is bounded by [`crate::comments::CommentStore::refresh_anchors`]:
    /// a comment lands on the matching line of the other side, or is marked outdated when
    /// there is no such line. Outdated is the honest answer — the note is about code this
    /// side does not have — and it is visible, which a silently misplaced comment is not.
    fn key(repo: &str, change_id: &str) -> String {
        format!("{repo}\u{1}{change_id}")
    }

    pub fn viewed(&self, repo: &str, change_id: &str) -> Vec<String> {
        self.data
            .viewed
            .get(&Self::key(repo, change_id))
            .map(|set| {
                let mut paths: Vec<String> = set.iter().cloned().collect();
                paths.sort();
                paths
            })
            .unwrap_or_default()
    }

    pub fn set_viewed(&mut self, repo: &str, change_id: &str, path: &str, viewed: bool) {
        let key = Self::key(repo, change_id);
        let set = self.data.viewed.entry(key.clone()).or_default();
        if viewed {
            set.insert(path.to_string());
        } else {
            set.remove(path);
        }
        if set.is_empty() {
            self.data.viewed.remove(&key);
        }
        self.persist();
    }

    pub fn reviewed_commit(&self, repo: &str, change_id: &str) -> Option<String> {
        self.data.reviewed.get(&Self::key(repo, change_id)).cloned()
    }

    pub fn mark_reviewed(&mut self, repo: &str, change_id: &str, commit_id: &str) {
        self.data
            .reviewed
            .insert(Self::key(repo, change_id), commit_id.to_string());
        self.persist();
    }

    pub fn walkthrough(&self, repo: &str, change_id: &str) -> Option<crate::walkthrough::Walkthrough> {
        self.data.walkthroughs.get(&Self::key(repo, change_id)).cloned()
    }

    pub fn set_walkthrough(
        &mut self,
        repo: &str,
        change_id: &str,
        walkthrough: crate::walkthrough::Walkthrough,
    ) {
        self.data
            .walkthroughs
            .insert(Self::key(repo, change_id), walkthrough);
        self.persist();
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string(&self.data) {
            Ok(json) => {
                if let Err(error) = std::fs::write(path, json) {
                    eprintln!(
                        "jjdiff: could not save review state to {}: {error}",
                        path.display()
                    );
                }
            }
            Err(error) => eprintln!("jjdiff: could not serialize review state: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("review.json");

        let mut store = ReviewStore::load(file.clone());
        store.set_viewed("/repo", "abcd", "src/main.rs", true);
        store.set_viewed("/repo", "abcd", "README.md", true);
        store.set_viewed("/repo", "abcd", "README.md", false);
        store.set_viewed("/repo", "wxyz", "other.txt", true);
        store.mark_reviewed("/repo", "abcd", "commit111");

        let reloaded = ReviewStore::load(file);
        assert_eq!(reloaded.viewed("/repo", "abcd"), vec!["src/main.rs"]);
        assert_eq!(reloaded.viewed("/repo", "wxyz"), vec!["other.txt"]);
        assert!(reloaded.viewed("/repo", "none").is_empty());
        // Different repo, same change id: isolated.
        assert!(reloaded.viewed("/elsewhere", "abcd").is_empty());
        assert_eq!(reloaded.reviewed_commit("/repo", "abcd").as_deref(), Some("commit111"));
        assert_eq!(reloaded.reviewed_commit("/repo", "wxyz"), None);
    }

    /// The two commits of a divergent change share their review state, on purpose.
    ///
    /// Pinned because it is a decision and not an accident, and because the change that
    /// would break it looks like a fix: adding the commit id to the key would make the
    /// sides independent and would throw away every note on the next rewrite of any change
    /// in any repo. See [`ReviewStore::key`] for the whole argument.
    ///
    /// There is nothing divergence-specific to set up here — that is the point. The store
    /// is handed a change id, both sides of a divergent change are the same change id, and
    /// so the same entry answers for both.
    #[test]
    fn both_commits_of_a_divergent_change_read_one_set_of_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = ReviewStore::load(tmp.path().join("review.json"));

        // Marked viewed while looking at one side...
        store.set_viewed("/repo", "diverged", "src/cache.rs", true);
        store.mark_reviewed("/repo", "diverged", "aaaa1111");
        // ...and the other side is the same change, so it reads the same answer.
        assert_eq!(store.viewed("/repo", "diverged"), vec!["src/cache.rs"]);

        // `reviewed_commit` is the half that still tells them apart: it stores a commit,
        // so "you last reviewed this at aaaa1111" stays true of the change and is simply
        // not this side's own id — which is what makes the other side read as changed.
        assert_eq!(store.reviewed_commit("/repo", "diverged").as_deref(), Some("aaaa1111"));
    }

    #[test]
    fn a_corrupt_file_is_set_aside_rather_than_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("review.json");
        std::fs::write(&file, "{ not json").unwrap();

        let mut store = ReviewStore::load(file.clone());
        store.set_viewed("/repo", "abcd", "src/main.rs", true);

        let rescued: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("review.bad."))
            .collect();
        assert_eq!(rescued.len(), 1, "the unparseable file should still be on disk");
        assert_eq!(std::fs::read_to_string(rescued[0].path()).unwrap(), "{ not json");
    }

    /// `repo\u{1}change_id` is a file format, not an implementation detail: a
    /// user's viewed flags and reviewed commits are already on disk under
    /// exactly these strings, and a store that spelled the key differently
    /// would read as an empty first run rather than as an error. Parsing a file
    /// written by hand is the point — a round-trip through this build would
    /// agree with whatever format it invented.
    #[test]
    fn reads_state_stored_under_the_documented_key() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("review.json");
        std::fs::write(
            &file,
            r#"{"viewed":{"/repo\u0001abcd":["src/main.rs"]},"reviewed":{"/repo\u0001abcd":"commit111"}}"#,
        )
        .unwrap();

        let store = ReviewStore::load(file);
        assert_eq!(store.viewed("/repo", "abcd"), vec!["src/main.rs"]);
        assert_eq!(store.reviewed_commit("/repo", "abcd").as_deref(), Some("commit111"));
    }

    #[test]
    fn marking_reviewed_again_overwrites() {
        let mut store = ReviewStore::default();
        store.mark_reviewed("/r", "c", "one");
        store.mark_reviewed("/r", "c", "two");
        assert_eq!(store.reviewed_commit("/r", "c").as_deref(), Some("two"));
    }
}
