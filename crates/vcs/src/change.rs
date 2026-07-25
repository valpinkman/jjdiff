use serde::{Deserialize, Serialize};

use crate::{Result, VcsError};

/// One revision as exposed to the app/UI. Identity is the **change id** — it survives
/// `describe`/`squash`/rebase, so review state keys on it (PLAN.md thesis #3).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub change_id: String,
    pub commit_id: String,
    pub parents: Vec<String>,
    pub description: String,
    pub author: Signature,
    pub committer: Signature,
    pub empty: bool,
    pub conflict: bool,
    pub immutable: bool,
    pub working_copy: bool,
    pub bookmarks: Vec<String>,
}

impl Change {
    pub fn first_line(&self) -> &str {
        self.description.lines().next().unwrap_or("")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// RFC 3339, as emitted by jj's `json()`.
    pub timestamp: String,
}

/// Wire format of `LOG_TEMPLATE` — the `commit` field is jj's own `json(self)`.
#[derive(Deserialize)]
struct LogRecord {
    commit: CommitInfo,
    empty: bool,
    conflict: bool,
    immutable: bool,
    working_copy: bool,
    bookmarks: Vec<String>,
}

#[derive(Deserialize)]
struct CommitInfo {
    commit_id: String,
    parents: Vec<String>,
    change_id: String,
    description: String,
    author: Signature,
    committer: Signature,
}

pub(crate) fn parse_record(line: &str) -> Result<Change> {
    let record: LogRecord = serde_json::from_str(line)
        .map_err(|error| VcsError::Parse(format!("bad log record: {error}; line: {line}")))?;
    let LogRecord { commit, empty, conflict, immutable, working_copy, bookmarks } = record;
    Ok(Change {
        change_id: commit.change_id,
        commit_id: commit.commit_id,
        parents: commit.parents,
        description: commit.description,
        author: commit.author,
        committer: commit.committer,
        empty,
        conflict,
        immutable,
        working_copy,
        bookmarks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured verbatim from `jj log -T <LOG_TEMPLATE>` with jj 0.43.0.
    const FIXTURE: &str = r#"{"commit":{"commit_id":"8bc80827823ad4678e7d24f8639413d3dd0c9333","parents":["f1bbe5aac1e4c795b25257abe60fdc8e0742a49e"],"change_id":"vzusolpoxkysuylvxpvpvyykzllnvtqt","description":"Switch frontend plan to Lit 3 (light-DOM code pane, CSS custom property theming)\n","author":{"name":"Valentin D. Pinkman","email":"v@example.com","timestamp":"2026-07-25T23:03:46+02:00"},"committer":{"name":"Valentin D. Pinkman","email":"v@example.com","timestamp":"2026-07-25T23:03:49+02:00"}},"empty":false,"conflict":false,"immutable":false,"working_copy":false,"bookmarks":[]}"#;

    #[test]
    fn parses_real_record() {
        let change = parse_record(FIXTURE).unwrap();
        assert_eq!(change.change_id, "vzusolpoxkysuylvxpvpvyykzllnvtqt");
        assert_eq!(change.commit_id, "8bc80827823ad4678e7d24f8639413d3dd0c9333");
        assert_eq!(change.parents.len(), 1);
        assert_eq!(change.first_line(), "Switch frontend plan to Lit 3 (light-DOM code pane, CSS custom property theming)");
        assert!(!change.working_copy);
        assert!(change.bookmarks.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_record("not json").is_err());
    }
}
