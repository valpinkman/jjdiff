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
    pub fn load(path: PathBuf) -> ReviewStore {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        ReviewStore { path: Some(path), data }
    }

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
        if let Ok(json) = serde_json::to_string(&self.data) {
            let _ = std::fs::write(path, json);
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

    #[test]
    fn marking_reviewed_again_overwrites() {
        let mut store = ReviewStore::default();
        store.mark_reviewed("/r", "c", "one");
        store.mark_reviewed("/r", "c", "two");
        assert_eq!(store.reviewed_commit("/r", "c").as_deref(), Some("two"));
    }
}
