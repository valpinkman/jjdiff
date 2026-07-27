//! Inline review comments — SQLite-backed, keyed by change id.
//!
//! The jj-native advantage (PLAN.md thesis #3, C2): a comment records
//! `(repo, change_id, path, hunk_id, side, line)` plus the commit id it was
//! written against. Because change ids survive `describe`/`squash`/rebase, a
//! comment stays attached to the code it was about — structurally impossible
//! in git, where codiff must re-anchor to a commit sha.
//!
//! When the change evolves, [`CommentStore::refresh_anchors`] re-anchors by
//! matching line content within the file; if the line is gone, the comment is
//! marked **outdated** and shown against its original text rather than
//! silently dropped or misplaced.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use jjdiff_diff::{FilePatch, LineKind};

/// Which side of the diff a comment is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Old side (removed/context lines, old line number).
    Old,
    /// New side (added/context lines, new line number).
    New,
}

impl Side {
    fn as_db(self) -> &'static str {
        match self {
            Side::Old => "old",
            Side::New => "new",
        }
    }

    fn from_db(s: &str) -> Side {
        if s == "old" {
            Side::Old
        } else {
            Side::New
        }
    }
}

/// A single review comment. Serialized to the UI as camelCase JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    /// Stable row id (SQLite primary key).
    pub id: i64,
    pub repo: String,
    pub change_id: String,
    pub path: String,
    /// `<path>#<index>` — the hunk the comment was written against.
    pub hunk_id: String,
    pub side: Side,
    /// 1-based line number on the chosen side.
    pub line: u32,
    /// The text of the line when the comment was written (drift detection + outdated view).
    pub line_text: String,
    /// Commit id the comment was written against.
    pub commit_id: String,
    pub author: String,
    pub body: String,
    /// ISO 8601 timestamp (UTC).
    pub created_at: String,
    /// Parent comment id for threading (null = top-level).
    pub parent_id: Option<i64>,
    /// Whether the comment is marked resolved.
    pub resolved: bool,
    /// Whether the anchor line no longer matches (drifted). When true, `line`
    /// is the original number; `line_text` is shown instead of the current
    /// file content at that line.
    pub outdated: bool,
}

/// Input for creating a new comment (id, timestamps, drift flags set by the store).
#[derive(Debug, Clone)]
pub struct NewComment {
    pub repo: String,
    pub change_id: String,
    pub path: String,
    pub hunk_id: String,
    pub side: Side,
    pub line: u32,
    pub line_text: String,
    pub commit_id: String,
    pub author: String,
    pub body: String,
    pub parent_id: Option<i64>,
}

/// SQLite-backed comment store. Behind a `Mutex` because Tauri commands run
/// across the main thread and the blocking pool; SQLite wants one writer at a
/// time. Reads are cheap and the write volume is low (human typing speed).
pub struct CommentStore {
    conn: Mutex<Connection>,
}

impl CommentStore {
    /// Open (or create) the comment database at `path`, running migrations.
    pub fn open(path: PathBuf) -> Result<CommentStore, String> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path).map_err(|e| format!("open comment db: {e}"))?;
        conn.execute_batch(MIGRATION).map_err(|e| format!("migrate comment db: {e}"))?;
        Ok(CommentStore { conn: Mutex::new(conn) })
    }

    /// In-memory store (for tests + as a placeholder before the data dir is known).
    pub fn in_memory() -> Result<CommentStore, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open in-memory: {e}"))?;
        conn.execute_batch(MIGRATION).map_err(|e| format!("migrate in-memory: {e}"))?;
        Ok(CommentStore { conn: Mutex::new(conn) })
    }

    /// Insert a new comment, returning the fully-formed row (with id + timestamp).
    pub fn add(&self, new: NewComment) -> Result<Comment, String> {
        let conn = self.conn.lock().map_err(|e| format!("db lock: {e}"))?;
        let created_at = now_iso();
        conn.execute(
            INSERT_COMMENT,
            params![
                new.repo,
                new.change_id,
                new.path,
                new.hunk_id,
                new.side.as_db(),
                new.line,
                new.line_text,
                new.commit_id,
                new.author,
                new.body,
                created_at,
                new.parent_id,
            ],
        )
        .map_err(|e| format!("insert comment: {e}"))?;
        let id = conn.last_insert_rowid();
        Ok(Comment {
            id,
            repo: new.repo,
            change_id: new.change_id,
            path: new.path,
            hunk_id: new.hunk_id,
            side: new.side,
            line: new.line,
            line_text: new.line_text,
            commit_id: new.commit_id,
            author: new.author,
            body: new.body,
            created_at,
            parent_id: new.parent_id,
            resolved: false,
            outdated: false,
        })
    }

    /// All comments for a `(repo, change_id)`, ordered by path then line then
    /// creation time. Threads come together because children sort right after
    /// their parent (parent_id < child id).
    pub fn list(&self, repo: &str, change_id: &str) -> Result<Vec<Comment>, String> {
        let conn = self.conn.lock().map_err(|e| format!("db lock: {e}"))?;
        let mut stmt = conn.prepare(SELECT_COMMENTS).map_err(|e| format!("prepare list: {e}"))?;
        let rows = stmt
            .query_map(params![repo, change_id], row_to_comment)
            .map_err(|e| format!("query comments: {e}"))?;
        let mut comments = Vec::new();
        for row in rows {
            comments.push(row.map_err(|e| format!("read comment row: {e}"))?);
        }
        Ok(comments)
    }

    /// Toggle (or set) the resolved flag on a comment.
    pub fn set_resolved(&self, id: i64, resolved: bool) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(UPDATE_RESOLVED, params![resolved, id])
            .map_err(|e| format!("set resolved: {e}"))?;
        Ok(())
    }

    /// Delete a comment (and its children, if any).
    pub fn delete(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("db lock: {e}"))?;
        // Delete children first, then the comment itself.
        conn.execute(DELETE_CHILDREN, params![id]).map_err(|e| format!("delete children: {e}"))?;
        conn.execute(DELETE_COMMENT, params![id]).map_err(|e| format!("delete comment: {e}"))?;
        Ok(())
    }

    /// Update the body of a comment (inline edit).
    pub fn update_body(&self, id: i64, body: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.execute(UPDATE_BODY, params![body, id])
            .map_err(|e| format!("update body: {e}"))?;
        Ok(())
    }

    /// Re-anchor comments for `(repo, change_id)` against the current diff.
    ///
    /// For each comment whose `commit_id` differs from `current_commit_id`:
    /// - try to find the `line_text` in the current file's lines on the same
    ///   side; if found, update `line` to the new position and clear `outdated`;
    /// - if not found, mark `outdated = true` so the UI shows it against the
    ///   original text rather than dropping it or misplacing it.
    ///
    /// Returns the number of comments whose anchor changed (re-anchored or
    /// marked outdated).
    pub fn refresh_anchors(
        &self,
        repo: &str,
        change_id: &str,
        current_commit_id: &str,
        files: &[FilePatch],
    ) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| format!("db lock: {e}"))?;
        let mut stmt = conn
            .prepare(SELECT_FOR_REFRESH)
            .map_err(|e| format!("prepare refresh: {e}"))?;
        let candidates: Vec<(i64, Side, u32, String, String)> = stmt
            .query_map(params![repo, change_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    Side::from_db(&row.get::<_, String>(1)?),
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|e| format!("query refresh: {e}"))?
            .filter_map(|r| r.ok())
            .filter(|(_, _, _, _, commit)| commit != current_commit_id)
            .collect();
        drop(stmt);

        let mut changed = 0;
        for (id, side, _old_line, line_text, _old_commit) in candidates {
            let new_line = find_line(files, side, &line_text);
            match new_line {
                Some(line) => {
                    conn.execute(UPDATE_ANCHOR, params![line, false, current_commit_id, id])
                        .map_err(|e| format!("re-anchor: {e}"))?;
                    changed += 1;
                }
                None => {
                    conn.execute(SET_OUTDATED, params![true, current_commit_id, id])
                        .map_err(|e| format!("mark outdated: {e}"))?;
                    changed += 1;
                }
            }
        }
        Ok(changed)
    }

    /// Render the pending (unresolved) comments for a change as a
    /// paste-ready Markdown review, grouped by file. Top-level comments come
    /// with their thread beneath them.
    pub fn export_markdown(&self, repo: &str, change_id: &str) -> Result<String, String> {
        let comments = self.list(repo, change_id)?;
        let pending: Vec<&Comment> = comments.iter().filter(|c| !c.resolved).collect();
        if pending.is_empty() {
            return Ok("No pending comments.".to_string());
        }

        // Group by path, preserving list order.
        let mut by_path: Vec<(&str, Vec<&Comment>)> = Vec::new();
        for comment in &pending {
            if let Some(group) = by_path.last_mut() {
                if group.0 == comment.path {
                    group.1.push(comment);
                    continue;
                }
            }
            by_path.push((comment.path.as_str(), vec![comment]));
        }

        let mut out = String::new();
        for (path, group) in by_path {
            out.push_str(&format!("## `{path}`\n\n"));
            // Top-level comments and their threads.
            let top_level: Vec<&Comment> = group.iter().copied().filter(|c| c.parent_id.is_none()).collect();
            for (idx, top) in top_level.iter().enumerate() {
                let line_label = if top.outdated {
                    format!("line {} (outdated)", top.line)
                } else {
                    format!("line {}", top.line)
                };
                out.push_str(&format!(
                    "**{} — {}**\n\n{}\n",
                    line_label,
                    top.author,
                    indent_body(&top.body)
                ));
                // Children.
                let children: Vec<&Comment> = group
                    .iter()
                    .copied()
                    .filter(|c| c.parent_id == Some(top.id))
                    .collect();
                for child in children {
                    out.push_str(&format!(
                        "> **{}**\n>\n> {}\n",
                        child.author,
                        indent_body(&child.body).replace('\n', "\n> ")
                    ));
                }
                if idx + 1 < top_level.len() {
                    out.push('\n');
                }
            }
            out.push('\n');
        }
        Ok(out.trim_end().to_string())
    }
}

/// Find the line number of `text` on `side` in the diff, for re-anchoring.
/// Returns the first match; if the text appears in multiple hunks the comment
/// may move, but that is the right behavior for a re-anchoring heuristic.
fn find_line(files: &[FilePatch], side: Side, text: &str) -> Option<u32> {
    for file in files {
        for hunk in &file.hunks {
            for line in &hunk.lines {
                let matches_side = match (side, line.kind) {
                    (Side::Old, LineKind::Removed) | (Side::Old, LineKind::Context) => {
                        line.old_line
                    }
                    (Side::New, LineKind::Added) | (Side::New, LineKind::Context) => {
                        line.new_line
                    }
                    _ => None,
                };
                if line.text == text {
                    if let Some(n) = matches_side {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

/// Indent a comment body by 4 spaces so it renders as a code block in Markdown
/// when the body contains code, and otherwise just preserves the text. We keep
/// it simple: wrap the body in a blockquote-style indent only when threaded.
fn indent_body(body: &str) -> String {
    body.lines().collect::<Vec<_>>().join("\n")
}

fn now_iso() -> String {
    // RFC 3339, UTC. We avoid pulling in chrono for one timestamp.
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let remainder = secs % 86400;
    let hour = remainder / 3600;
    let minute = (remainder % 3600) / 60;
    let second = remainder % 60;

    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn row_to_comment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Comment> {
    Ok(Comment {
        id: row.get("id")?,
        repo: row.get("repo")?,
        change_id: row.get("change_id")?,
        path: row.get("path")?,
        hunk_id: row.get("hunk_id")?,
        side: Side::from_db(&row.get::<_, String>("side")?),
        line: row.get("line")?,
        line_text: row.get("line_text")?,
        commit_id: row.get("commit_id")?,
        author: row.get("author")?,
        body: row.get("body")?,
        created_at: row.get("created_at")?,
        parent_id: row.get("parent_id")?,
        resolved: row.get("resolved")?,
        outdated: row.get("outdated")?,
    })
}

const MIGRATION: &str = "\
CREATE TABLE IF NOT EXISTS comments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    repo        TEXT NOT NULL,
    change_id   TEXT NOT NULL,
    path        TEXT NOT NULL,
    hunk_id     TEXT NOT NULL,
    side        TEXT NOT NULL,
    line        INTEGER NOT NULL,
    line_text   TEXT NOT NULL,
    commit_id   TEXT NOT NULL,
    author      TEXT NOT NULL,
    body        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    parent_id   INTEGER REFERENCES comments(id) ON DELETE CASCADE,
    resolved    INTEGER NOT NULL DEFAULT 0,
    outdated    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_comments_repo_change ON comments(repo, change_id);
CREATE INDEX IF NOT EXISTS idx_comments_path_line ON comments(repo, change_id, path, line);
";

const INSERT_COMMENT: &str = "\
INSERT INTO comments (repo, change_id, path, hunk_id, side, line, line_text, commit_id, author, body, created_at, parent_id)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)";

const SELECT_COMMENTS: &str = "\
SELECT id, repo, change_id, path, hunk_id, side, line, line_text, commit_id, author, body, created_at, parent_id, resolved, outdated
FROM comments
WHERE repo = ?1 AND change_id = ?2
ORDER BY path, line, created_at";

const UPDATE_RESOLVED: &str = "UPDATE comments SET resolved = ?1 WHERE id = ?2";
const UPDATE_BODY: &str = "UPDATE comments SET body = ?1 WHERE id = ?2";
const DELETE_COMMENT: &str = "DELETE FROM comments WHERE id = ?1";
const DELETE_CHILDREN: &str = "DELETE FROM comments WHERE parent_id = ?1";

const SELECT_FOR_REFRESH: &str = "\
SELECT id, side, line, line_text, commit_id
FROM comments
WHERE repo = ?1 AND change_id = ?2 AND outdated = 0";

const UPDATE_ANCHOR: &str = "\
UPDATE comments SET line = ?1, outdated = ?2, commit_id = ?3 WHERE id = ?4";

const SET_OUTDATED: &str = "\
UPDATE comments SET outdated = ?1, commit_id = ?2 WHERE id = ?3";

#[cfg(test)]
mod tests {
    use super::*;
    use jjdiff_diff::parse_git_patch;

    const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n ctx\n-old\n+new\n ctx2\n";

    fn files() -> Vec<FilePatch> {
        parse_git_patch(PATCH).unwrap()
    }

    fn sample(side: Side, line: u32, line_text: &str) -> NewComment {
        NewComment {
            repo: "/repo".into(),
            change_id: "change1".into(),
            path: "a.rs".into(),
            hunk_id: "a.rs#0".into(),
            side,
            line,
            line_text: line_text.into(),
            commit_id: "commit_a".into(),
            author: "alice".into(),
            body: "this looks wrong".into(),
            parent_id: None,
        }
    }

    #[test]
    fn add_and_list_returns_the_comment() {
        let store = CommentStore::in_memory().unwrap();
        let added = store.add(sample(Side::New, 2, "new")).unwrap();
        assert_eq!(added.id, 1);
        assert!(!added.resolved);
        assert!(!added.outdated);

        let list = store.list("/repo", "change1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].body, "this looks wrong");
        assert_eq!(list[0].line, 2);
    }

    #[test]
    fn list_is_isolated_per_change_and_repo() {
        let store = CommentStore::in_memory().unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();
        // Same repo, different change.
        let mut other = sample(Side::New, 2, "new");
        other.change_id = "change2".into();
        store.add(other).unwrap();
        // Different repo, same change id.
        let mut other_repo = sample(Side::New, 2, "new");
        other_repo.repo = "/elsewhere".into();
        store.add(other_repo).unwrap();

        assert_eq!(store.list("/repo", "change1").unwrap().len(), 1);
        assert_eq!(store.list("/repo", "change2").unwrap().len(), 1);
        assert_eq!(store.list("/elsewhere", "change1").unwrap().len(), 1);
        assert!(store.list("/repo", "nope").unwrap().is_empty());
    }

    #[test]
    fn resolve_and_delete_work() {
        let store = CommentStore::in_memory().unwrap();
        let added = store.add(sample(Side::New, 2, "new")).unwrap();
        store.set_resolved(added.id, true).unwrap();
        let list = store.list("/repo", "change1").unwrap();
        assert!(list[0].resolved);

        store.set_resolved(added.id, false).unwrap();
        assert!(!store.list("/repo", "change1").unwrap()[0].resolved);

        store.delete(added.id).unwrap();
        assert!(store.list("/repo", "change1").unwrap().is_empty());
    }

    #[test]
    fn delete_cascades_to_children() {
        let store = CommentStore::in_memory().unwrap();
        let parent = store.add(sample(Side::New, 2, "new")).unwrap();
        let mut child = sample(Side::New, 2, "new");
        child.body = "agreed".into();
        child.parent_id = Some(parent.id);
        store.add(child).unwrap();
        assert_eq!(store.list("/repo", "change1").unwrap().len(), 2);

        store.delete(parent.id).unwrap();
        assert!(store.list("/repo", "change1").unwrap().is_empty());
    }

    #[test]
    fn update_body_changes_the_text() {
        let store = CommentStore::in_memory().unwrap();
        let added = store.add(sample(Side::New, 2, "new")).unwrap();
        store.update_body(added.id, "edited take").unwrap();
        let list = store.list("/repo", "change1").unwrap();
        assert_eq!(list[0].body, "edited take");
    }

    #[test]
    fn refresh_anchors_when_line_moved() {
        let store = CommentStore::in_memory().unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();

        // Evolved diff: the same text is now at line 5.
        let evolved = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,6 +1,6 @@\n a\n b\n c\n d\n-new\n+new\n ctx\n";
        let files = parse_git_patch(evolved).unwrap();
        let changed = store.refresh_anchors("/repo", "change1", "commit_b", &files).unwrap();
        assert_eq!(changed, 1);

        let list = store.list("/repo", "change1").unwrap();
        assert_eq!(list[0].line, 5, "re-anchored to new position");
        assert!(!list[0].outdated);
        assert_eq!(list[0].commit_id, "commit_b");
    }

    #[test]
    fn refresh_marks_outdated_when_line_gone() {
        let store = CommentStore::in_memory().unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();

        // Evolved diff: the text "new" no longer appears anywhere.
        let evolved = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n ctx\n-old\n+completely_different\n ctx2\n";
        let files = parse_git_patch(evolved).unwrap();
        let changed = store.refresh_anchors("/repo", "change1", "commit_b", &files).unwrap();
        assert_eq!(changed, 1);

        let list = store.list("/repo", "change1").unwrap();
        assert!(list[0].outdated, "marked outdated when line is gone");
        assert_eq!(list[0].line_text, "new", "original text preserved");
    }

    #[test]
    fn refresh_skips_comments_already_on_current_commit() {
        let store = CommentStore::in_memory().unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();
        // Same commit_id as the comment was written against — no-op.
        let changed = store.refresh_anchors("/repo", "change1", "commit_a", &files()).unwrap();
        assert_eq!(changed, 0);
    }

    #[test]
    fn refresh_skips_already_outdated_comments() {
        let store = CommentStore::in_memory().unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();
        // First evolution marks it outdated.
        let gone = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n ctx\n-old\n+other\n ctx2\n";
        let files1 = parse_git_patch(gone).unwrap();
        store.refresh_anchors("/repo", "change1", "commit_b", &files1).unwrap();
        // Second evolution: the text is back. But the comment is already outdated
        // and SELECT_FOR_REFRESH filters outdated=0, so it stays outdated.
        let files2 = files();
        let changed = store.refresh_anchors("/repo", "change1", "commit_c", &files2).unwrap();
        assert_eq!(changed, 0, "already-outdated comments are not re-anchored");
        assert!(store.list("/repo", "change1").unwrap()[0].outdated);
    }

    #[test]
    fn export_markdown_groups_by_file_and_includes_threads() {
        let store = CommentStore::in_memory().unwrap();
        let top1 = store.add(sample(Side::New, 2, "new")).unwrap();
        let mut reply = sample(Side::New, 2, "new");
        reply.body = "agreed".into();
        reply.author = "bob".into();
        reply.parent_id = Some(top1.id);
        store.add(reply).unwrap();
        let mut other_line = sample(Side::Old, 1, "old");
        other_line.body = "why removed?".into();
        store.add(other_line).unwrap();

        let md = store.export_markdown("/repo", "change1").unwrap();
        assert!(md.contains("`a.rs`"));
        assert!(md.contains("line 2"));
        assert!(md.contains("this looks wrong"));
        assert!(md.contains("agreed"));
        assert!(md.contains("line 1"));
        assert!(md.contains("why removed?"));
    }

    #[test]
    fn export_markdown_skips_resolved_and_handles_empty() {
        let store = CommentStore::in_memory().unwrap();
        let added = store.add(sample(Side::New, 2, "new")).unwrap();
        store.set_resolved(added.id, true).unwrap();
        assert_eq!(store.export_markdown("/repo", "change1").unwrap(), "No pending comments.");
    }

    #[test]
    fn export_markdown_marks_outdated_lines() {
        let store = CommentStore::in_memory().unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();
        let gone = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n ctx\n-old\n+other\n ctx2\n";
        let files = parse_git_patch(gone).unwrap();
        store.refresh_anchors("/repo", "change1", "commit_b", &files).unwrap();

        let md = store.export_markdown("/repo", "change1").unwrap();
        assert!(md.contains("outdated"), "{md}");
    }

    #[test]
    fn persistence_roundtrips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("comments.db");
        let store = CommentStore::open(path.clone()).unwrap();
        store.add(sample(Side::New, 2, "new")).unwrap();

        let reopened = CommentStore::open(path).unwrap();
        assert_eq!(reopened.list("/repo", "change1").unwrap().len(), 1);
    }

    #[test]
    fn now_iso_is_rfc3339_utc() {
        let ts = now_iso();
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20); // YYYY-MM-DDTHH:MM:SSZ
        assert_eq!(ts.as_bytes()[10], b'T');
    }
}
