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

/// How a local bookmark stands against one remote it tracks.
///
/// **The counts are stated from the local bookmark's point of view**, which is the
/// inverse of the template keywords they come from. `jj bookmark list` renders a
/// remote ref's own perspective ("@origin (behind by 2 commits)" means the *remote*
/// lags), but a reviewer is asking about their own branch: what would a push send,
/// what would a fetch bring. See [`crate::Repo::bookmark_statuses`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkStatus {
    pub name: String,
    pub remote: String,
    /// Commits the local bookmark has that the remote does not — what a push would send.
    pub ahead: u32,
    /// Commits the remote has that the local does not — what a fetch would bring.
    pub behind: u32,
}

pub(crate) fn parse_bookmark_status(line: &str) -> Result<BookmarkStatus> {
    serde_json::from_str(line)
        .map_err(|error| VcsError::Parse(format!("bad bookmark record: {error}; line: {line}")))
}

/// One predecessor version of a change, from `jj evolog`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvologEntry {
    pub commit_id: String,
    pub change_id: String,
    pub description: String,
    /// Committer timestamp (RFC 3339) — when this version of the change was written.
    pub timestamp: String,
}

pub(crate) fn parse_evolog_record(line: &str) -> Result<EvologEntry> {
    let commit: CommitInfo = serde_json::from_str(line)
        .map_err(|error| VcsError::Parse(format!("bad evolog record: {error}; line: {line}")))?;
    Ok(EvologEntry {
        commit_id: commit.commit_id,
        change_id: commit.change_id,
        description: commit.description,
        timestamp: commit.committer.timestamp,
    })
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

/// One entry from `jj op log`. jj's `json(self)` gives us everything, including the exact
/// argv that produced the operation — no output parsing anywhere.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: String,
    pub description: String,
    /// Literal command line, when jj recorded one (snapshots have none).
    pub args: Option<String>,
    /// RFC 3339 start time.
    pub time: String,
    pub user: String,
    /// Working-copy snapshots are noise in a user-facing log.
    pub snapshot: bool,
}

#[derive(Deserialize)]
struct OperationRecord {
    id: String,
    description: String,
    time: OperationTime,
    username: String,
    #[serde(default)]
    is_snapshot: bool,
    #[serde(default)]
    attributes: OperationAttributes,
}

#[derive(Deserialize)]
struct OperationTime {
    start: String,
}

#[derive(Deserialize, Default)]
struct OperationAttributes {
    #[serde(default)]
    args: Option<String>,
}

pub(crate) fn parse_operation(line: &str) -> Result<Operation> {
    let record: OperationRecord = serde_json::from_str(line)
        .map_err(|error| VcsError::Parse(format!("bad op record: {error}; line: {line}")))?;
    Ok(Operation {
        id: record.id,
        description: record.description,
        args: record.attributes.args,
        time: record.time.start,
        user: record.username,
        snapshot: record.is_snapshot,
    })
}

#[cfg(test)]
mod operation_tests {
    use super::*;

    // Captured verbatim from `jj op log -T 'json(self)'` (jj 0.43.0).
    const FIXTURE: &str = r#"{"id":"8ac573e89157","parents":["0d1e90d292e0"],"time":{"start":"2026-07-26T21:02:00.742+02:00","end":"2026-07-26T21:02:11.042+02:00"},"description":"push all deleted bookmarks/tags to git remote origin","hostname":"host.local","username":"valpinkman","is_snapshot":false,"workspace_name":"default","attributes":{"args":"jj git push --remote origin --deleted"}}"#;

    #[test]
    fn parses_operation_record() {
        let op = parse_operation(FIXTURE).unwrap();
        assert_eq!(op.id, "8ac573e89157");
        assert_eq!(op.description, "push all deleted bookmarks/tags to git remote origin");
        assert_eq!(op.args.as_deref(), Some("jj git push --remote origin --deleted"));
        assert!(op.time.starts_with("2026-07-26"));
        assert_eq!(op.user, "valpinkman");
        assert!(!op.snapshot);
    }

    #[test]
    fn tolerates_missing_attributes() {
        // Snapshot operations carry no argv.
        let raw = r#"{"id":"abc","parents":[],"time":{"start":"2026-01-01T00:00:00+00:00","end":"2026-01-01T00:00:01+00:00"},"description":"snapshot working copy","hostname":"h","username":"u","is_snapshot":true}"#;
        let op = parse_operation(raw).unwrap();
        assert!(op.args.is_none());
        assert!(op.snapshot);
    }
}
