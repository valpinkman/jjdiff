//! Viewed-file flags, keyed `(repo root, change id, path)`.
//!
//! Keying on **change ids** is the point (PLAN.md thesis #3): flags survive `describe`,
//! `squash`, and rebases, and after `jj new` the flags stay with the change you reviewed.
//! JSON-file persistence is deliberate for this scale; a database arrives with review
//! comments (M3+).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Default)]
pub struct ViewedStore {
    path: Option<PathBuf>,
    /// `repo\u{1}change_id` → viewed paths.
    map: HashMap<String, HashSet<String>>,
}

impl ViewedStore {
    pub fn load(path: PathBuf) -> ViewedStore {
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        ViewedStore { path: Some(path), map }
    }

    fn key(repo: &str, change_id: &str) -> String {
        format!("{repo}\u{1}{change_id}")
    }

    pub fn viewed(&self, repo: &str, change_id: &str) -> Vec<String> {
        self.map
            .get(&Self::key(repo, change_id))
            .map(|set| {
                let mut paths: Vec<String> = set.iter().cloned().collect();
                paths.sort();
                paths
            })
            .unwrap_or_default()
    }

    pub fn set(&mut self, repo: &str, change_id: &str, path: &str, viewed: bool) {
        let key = Self::key(repo, change_id);
        let set = self.map.entry(key.clone()).or_default();
        if viewed {
            set.insert(path.to_string());
        } else {
            set.remove(path);
        }
        if set.is_empty() {
            self.map.remove(&key);
        }
        self.persist();
    }

    fn persist(&self) {
        let Some(path) = &self.path else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string(&self.map) {
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
        let file = tmp.path().join("viewed.json");

        let mut store = ViewedStore::load(file.clone());
        store.set("/repo", "abcd", "src/main.rs", true);
        store.set("/repo", "abcd", "README.md", true);
        store.set("/repo", "abcd", "README.md", false);
        store.set("/repo", "wxyz", "other.txt", true);

        let reloaded = ViewedStore::load(file);
        assert_eq!(reloaded.viewed("/repo", "abcd"), vec!["src/main.rs"]);
        assert_eq!(reloaded.viewed("/repo", "wxyz"), vec!["other.txt"]);
        assert!(reloaded.viewed("/repo", "none").is_empty());
        // Different repo, same change id: isolated.
        assert!(reloaded.viewed("/elsewhere", "abcd").is_empty());
    }
}
