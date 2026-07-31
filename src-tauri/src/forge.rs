//! Forge review through the `gh` CLI.
//!
//! Discipline (PLAN.md, C4): scoped to the **CLIs, not the REST APIs**, so auth
//! is someone else's problem — we never see a token, never refresh one, and
//! never ship per-forge HTTP code. Every call is one headless invocation
//! returning JSON, in the same spirit as [`crate::walkthrough`]'s agent
//! backends.
//!
//! The forge is inferred from the remote URL rather than configured: a
//! colocated repo already knows where it pushes.
//!
//! GitHub only. A GitLab path existed and was removed: it was written against
//! `glab`'s documented JSON and never run against a live instance, so it was
//! shipping the *appearance* of support. `Kind` stays an enum so adding one
//! back is a variant rather than a rewrite, and `from_remote` still refuses a
//! host it cannot place instead of guessing.

use std::process::Command;

use serde::{Deserialize, Serialize};

/// Which forge CLI backs a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    GitHub,
}

impl Kind {
    /// Infer the forge from a git remote URL, in either SSH or HTTPS form.
    /// Self-hosted instances are recognised by hostname convention only —
    /// there is no reliable probe short of talking to the server.
    pub fn from_remote(url: &str) -> Option<Kind> {
        let host = remote_host(url)?.to_ascii_lowercase();
        (host == "github.com" || host.starts_with("github.")).then_some(Kind::GitHub)
    }

    /// The CLI that drives this forge.
    pub fn binary(self) -> &'static str {
        match self {
            Kind::GitHub => "gh",
        }
    }

    /// What this forge calls a change proposal, for user-facing strings.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::GitHub => "pull request",
        }
    }

    /// The remote ref holding a proposal's head commit. GitHub publishes one
    /// per pull request, which is what makes reviewing without a fork remote
    /// possible.
    pub fn head_ref(self, number: u32) -> String {
        match self {
            Kind::GitHub => format!("refs/pull/{number}/head"),
        }
    }

    /// Local bookmark a fetched head lands on. Namespaced so it is obvious
    /// where it came from and safe to delete.
    pub fn local_bookmark(self, number: u32) -> String {
        match self {
            Kind::GitHub => format!("jjdiff-pr-{number}"),
        }
    }
}

/// Extract the hostname from `git@host:owner/repo.git`, `ssh://git@host/…` or
/// `https://host/…`.
fn remote_host(url: &str) -> Option<&str> {
    let url = url.trim();
    if let Some(rest) = url.split_once("://").map(|(_, rest)| rest) {
        // scheme://[user@]host[:port]/path
        let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
        let host = rest.split(['/', ':']).next()?;
        return (!host.is_empty()).then_some(host);
    }
    // scp-like: [user@]host:path
    let rest = url.rsplit_once('@').map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u32,
    pub title: String,
    pub body: String,
    pub author: String,
    /// Branch being merged into.
    pub base: String,
    /// Branch being merged from.
    pub head: String,
    /// Commit the forge diffs *against* — its own merge base. Reviewing has to
    /// use this rather than `base..head`: once a proposal is merged its head is
    /// an ancestor of the base branch, so a branch-name revset goes empty and
    /// the review shows nothing. The forge remembers the right commit.
    pub base_oid: String,
    /// Commit at the tip of the proposal.
    pub head_oid: String,
    /// OPEN / MERGED / CLOSED.
    pub state: String,
    pub draft: bool,
    /// Forge's mergeability verdict; "UNKNOWN" until it finishes computing.
    pub mergeable: String,
    pub url: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    pub reviewers: Vec<Reviewer>,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reviewer {
    pub name: String,
    /// REQUESTED / APPROVED / CHANGES_REQUESTED / COMMENTED.
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    /// QUEUED / IN_PROGRESS / COMPLETED.
    pub status: String,
    /// SUCCESS / FAILURE / SKIPPED / …; empty while still running.
    pub conclusion: String,
    pub url: String,
}

/// One row in the "open proposals" list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub number: u32,
    pub title: String,
    pub author: String,
    pub state: String,
    pub draft: bool,
    pub head: String,
    pub updated_at: String,
}

/// What a submitted review says.
///
/// camelCase, not lowercase: the spelling here is the wire format `ui/src/ipc.ts`
/// declares, and `lowercase` would run `RequestChanges` together into
/// `requestchanges` — a verdict the UI could never submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Approve,
    RequestChanges,
    Comment,
}

impl Verdict {
    fn gh_flag(self) -> &'static str {
        match self {
            Verdict::Approve => "--approve",
            Verdict::RequestChanges => "--request-changes",
            Verdict::Comment => "--comment",
        }
    }

    /// The `event` value of GitHub's create-review API.
    fn gh_event(self) -> &'static str {
        match self {
            Verdict::Approve => "APPROVE",
            Verdict::RequestChanges => "REQUEST_CHANGES",
            Verdict::Comment => "COMMENT",
        }
    }
}

/// A proposal to open. The fields the compose dialog collects, and nothing the
/// forge can work out for itself.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPullRequest {
    pub title: String,
    pub body: String,
    /// Branch to merge into.
    pub base: String,
    /// Branch to merge from. Must already be on the remote — the caller pushes
    /// first, because `gh` resolves this against what the forge can see.
    pub head: String,
    pub draft: bool,
}

/// A proposal that now exists.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    /// Read out of the URL the forge printed. `None` if it was not in the shape
    /// we expect, which is **not** an error: the proposal is open either way, and
    /// failing here would report a success as a failure. The banner finds it on
    /// the next refresh regardless — the branch now has a proposal, which is the
    /// only question `find_by_head` asks.
    pub number: Option<u32>,
    pub url: String,
}

/// One inline comment to post against a line of the proposal's diff.
///
/// This is the payoff of anchoring comments on change ids (C2): the comments a
/// reviewer wrote while reading land on the forge as real line comments rather
/// than a wall of Markdown quoting line numbers.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub path: String,
    pub line: u32,
    /// "old" or "new" — jjdiff's vocabulary, mapped per forge.
    pub side: String,
    pub body: String,
}

impl ReviewComment {
    /// GitHub names the diff sides LEFT (pre-image) and RIGHT (post-image).
    fn github_side(&self) -> &'static str {
        if self.side == "old" { "LEFT" } else { "RIGHT" }
    }
}

pub struct Client {
    pub kind: Kind,
    /// Repo root; every CLI call runs here so the tool resolves the same repo
    /// jjdiff has open, regardless of the process's cwd.
    pub root: std::path::PathBuf,
    /// The git remote this client was built from, so a fetch goes to the same
    /// place the client queried rather than re-running the selection.
    pub remote: String,
}

impl Client {
    /// Why the CLI could not be started. Shared by both spawn paths: a missing
    /// `gh` is the single most likely failure here and the one with an
    /// actionable answer, so "install it and log in" must not depend on which
    /// helper the call happened to use.
    fn spawn_error(&self, error: &std::io::Error) -> String {
        let binary = self.kind.binary();
        match error.kind() {
            std::io::ErrorKind::NotFound => format!(
                "`{binary}` was not found on PATH — jjdiff reviews {}s through the \
                 {binary} CLI, so install it and run `{binary} auth login`",
                self.kind.noun()
            ),
            _ => format!("cannot run `{binary}`: {error}"),
        }
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        let binary = self.kind.binary();
        let output = Command::new(binary)
            .args(args)
            .current_dir(&self.root)
            .output()
            .map_err(|error| self.spawn_error(&error))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!("{binary} {}: {stderr}", args.join(" ")));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn pull_request(&self, number: u32) -> Result<PullRequest, String> {
        let number_arg = number.to_string();
        match self.kind {
            Kind::GitHub => {
                const FIELDS: &str = "number,title,body,author,baseRefName,headRefName,state,\
                                      isDraft,mergeable,url,additions,deletions,changedFiles,\
                                      reviewRequests,reviews,statusCheckRollup,baseRefOid,\
                                      headRefOid";
                let raw = self.run(&["pr", "view", &number_arg, "--json", FIELDS])?;
                github::parse_pull_request(&raw)
            }
        }
    }

    pub fn list(&self, limit: u32) -> Result<Vec<Summary>, String> {
        let limit = limit.to_string();
        match self.kind {
            Kind::GitHub => {
                let raw = self.run(&[
                    "pr",
                    "list",
                    "--limit",
                    &limit,
                    "--json",
                    "number,title,author,state,isDraft,headRefName,updatedAt",
                ])?;
                github::parse_list(&raw)
            }
        }
    }

    /// The proposal whose head is `branch`, asked for by name.
    ///
    /// [`list`](Self::list) answers "what is open here" and is the wrong tool for
    /// "is there a proposal for *this* branch": it returns one page of open
    /// proposals, so on a repo with more of them than the page holds — 200+ open
    /// against a limit of 30, on the monorepo that turned this up — a branch's
    /// own proposal is routinely outside the window, and a merged one is never
    /// in it at all. The banner could not appear there however long you waited.
    ///
    /// `--head` asks the forge the exact question instead, so the answer does
    /// not depend on how busy the repo is, and `--state all` covers a proposal
    /// that has already landed — which is still worth a banner, since a merged
    /// PR is the review that happened. Newest first, `--limit 1`: a branch
    /// reused across several proposals means the current one.
    pub fn find_by_head(&self, branch: &str) -> Result<Option<Summary>, String> {
        match self.kind {
            Kind::GitHub => {
                let raw = self.run(&[
                    "pr",
                    "list",
                    "--head",
                    branch,
                    "--state",
                    "all",
                    "--limit",
                    "1",
                    "--json",
                    "number,title,author,state,isDraft,headRefName,updatedAt",
                ])?;
                Ok(github::parse_list(&raw)?.into_iter().next())
            }
        }
    }

    /// The whole conversation on a proposal, oldest first.
    ///
    /// Two calls, because GitHub does not expose these together: `pr view`
    /// carries issue comments and reviews, while comments anchored to a line
    /// only come from the REST endpoint. The inline call is allowed to fail on
    /// its own — a proposal whose discussion loads but whose line comments do
    /// not is far better than no conversation at all.
    pub fn activity(&self, number: u32) -> Result<Vec<Activity>, String> {
        let number_arg = number.to_string();
        let raw = self.run(&["pr", "view", &number_arg, "--json", "comments,reviews"])?;
        let mut entries = github::parse_activity(&raw)?;

        let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{number_arg}/comments");
        if let Ok(raw) = self.run(&["api", "--paginate", &endpoint]) {
            entries.extend(github::parse_inline_comments(&raw).unwrap_or_default());
        }

        entries.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(entries)
    }

    /// The branch a proposal should target unless told otherwise.
    ///
    /// Asked of the forge rather than guessed from the local repo. `main` is only
    /// a convention, jj's `trunk()` resolves against whatever is *fetched* here,
    /// and neither knows that a repo's default was changed on the server. Getting
    /// this wrong opens a proposal against the wrong branch, which is public and
    /// annoying to correct, so it is worth the call.
    pub fn default_branch(&self) -> Result<String, String> {
        match self.kind {
            Kind::GitHub => {
                let raw = self.run(&["repo", "view", "--json", "defaultBranchRef"])?;
                github::parse_default_branch(&raw)
            }
        }
    }

    /// Open a proposal.
    ///
    /// Outward-facing and not undoable from here — the caller confirms, and the
    /// head branch is already pushed by the time this runs.
    ///
    /// The body goes on **stdin**, not in an argument. A description is
    /// arbitrary user text of unbounded length: as an argv entry it runs into
    /// `ARG_MAX` on a long one, and every backtick and `$` in it is one
    /// misplaced shell away from being executed. `--body-file -` has neither
    /// problem and is what `gh` documents for exactly this.
    pub fn create(&self, request: &NewPullRequest) -> Result<Created, String> {
        match self.kind {
            Kind::GitHub => {
                let mut args = vec![
                    "pr",
                    "create",
                    "--base",
                    &request.base,
                    "--head",
                    &request.head,
                    "--title",
                    &request.title,
                    "--body-file",
                    "-",
                ];
                if request.draft {
                    args.push("--draft");
                }
                let out = self.run_with_stdin(&args, &request.body)?;
                let url = out
                    .split_whitespace()
                    .rev()
                    .find(|token| token.starts_with("http"))
                    .ok_or_else(|| {
                        format!("gh pr create said nothing that looks like a URL: {}", out.trim())
                    })?
                    .to_string();
                Ok(Created { number: pull_request_number(&url), url })
            }
        }
    }

    /// Like [`Self::run`], but feeds `stdin` — `gh api --input -` wants its
    /// JSON body there, and nested arrays cannot be expressed with `-f` flags.
    fn run_with_stdin(&self, args: &[&str], stdin: &str) -> Result<String, String> {
        use std::io::Write;
        let binary = self.kind.binary();
        let mut child = Command::new(binary)
            .args(args)
            .current_dir(&self.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| self.spawn_error(&error))?;
        child
            .stdin
            .take()
            .ok_or_else(|| format!("`{binary}` has no stdin"))?
            .write_all(stdin.as_bytes())
            .map_err(|error| format!("cannot write to `{binary}`: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("`{binary}` failed: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Submit a review. Outward-facing and effectively irreversible — the
    /// caller confirms before reaching here.
    ///
    /// With inline `comments`, this goes through the create-review API so they
    /// land as real line comments. That call is all-or-nothing: GitHub rejects
    /// the *whole* review if any comment names a line outside the diff, which
    /// is easy to hit when a comment was written against a line the proposal
    /// does not touch. Rather than lose a reviewer's work to a 422, we retry
    /// with the comments folded into the body and report the downgrade.
    pub fn submit_review(
        &self,
        number: u32,
        verdict: Verdict,
        body: &str,
        comments: &[ReviewComment],
    ) -> Result<Submitted, String> {
        let number_arg = number.to_string();
        match self.kind {
            Kind::GitHub if !comments.is_empty() => {
                let payload = serde_json::json!({
                    "event": verdict.gh_event(),
                    "body": body,
                    "comments": comments
                        .iter()
                        .map(|comment| serde_json::json!({
                            "path": comment.path,
                            "line": comment.line,
                            "side": comment.github_side(),
                            "body": comment.body,
                        }))
                        .collect::<Vec<_>>(),
                });
                let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{number_arg}/reviews");
                let args = ["api", "--method", "POST", &endpoint, "--input", "-"];
                match self.run_with_stdin(&args, &payload.to_string()) {
                    Ok(_) => Ok(Submitted { inline: comments.len(), fell_back: None }),
                    Err(error) => {
                        // Fold the comments into the body and try the plain path.
                        let merged = merge_comments_into_body(body, comments);
                        self.submit_review(number, verdict, &merged, &[])?;
                        Ok(Submitted { inline: 0, fell_back: Some(first_line(&error)) })
                    }
                }
            }
            Kind::GitHub => {
                let mut args = vec!["pr", "review", &number_arg, verdict.gh_flag()];
                // `--approve` with an empty body is valid; `--comment` is not.
                if !body.trim().is_empty() {
                    args.extend(["--body", body]);
                }
                self.run(&args)?;
                Ok(Submitted { inline: 0, fell_back: None })
            }
        }
    }
}

/// One entry in a proposal's conversation.
///
/// GitHub keeps these in three places that have to be merged to read as one
/// thread: issue comments (the discussion at the bottom), reviews (a verdict
/// with an optional body) and review comments (anchored to a file and line).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    /// `comment`, `review` or `inline` — which of the three it came from.
    pub kind: &'static str,
    pub author: String,
    pub body: String,
    /// RFC 3339, and the only thing that puts the three sources in order.
    pub created_at: String,
    /// Review verdict (`APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`); empty
    /// for anything that is not a review.
    pub state: String,
    /// Inline only: where the comment hangs.
    pub path: String,
    pub line: u32,
    pub url: String,
}

/// What a submitted review actually did, so the UI can be honest about it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Submitted {
    /// How many comments landed as real inline comments.
    pub inline: usize,
    /// Set when inline posting failed and the comments went into the body
    /// instead; carries the forge's reason.
    pub fell_back: Option<String>,
}

fn first_line(text: &str) -> String {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or(text).trim().to_string()
}

/// The proposal number in a forge URL — the trailing path segment after `pull`
/// (GitHub) or `merge_requests` (the shape a restored GitLab would print).
///
/// Deliberately reads the segment *after the keyword* rather than the last
/// number in the string: `…/pull/12/files#discussion_r34` ends in neither, and
/// an owner or repo with digits in its name sits earlier in the same path.
fn pull_request_number(url: &str) -> Option<u32> {
    let mut segments = url.split('/');
    while let Some(segment) = segments.next() {
        if segment == "pull" || segment == "pulls" || segment == "merge_requests" {
            return segments.next()?.parse().ok();
        }
    }
    None
}

/// Render comments as Markdown beneath `body` — the fallback when they cannot
/// be anchored. Losing them silently would be the worst outcome of the three.
fn merge_comments_into_body(body: &str, comments: &[ReviewComment]) -> String {
    if comments.is_empty() {
        return body.to_string();
    }
    let mut out = body.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    for comment in comments {
        out.push_str(&format!(
            "**`{}`** line {} ({})\n\n{}\n\n",
            comment.path,
            comment.line,
            if comment.side == "old" { "before" } else { "after" },
            comment.body.trim()
        ));
    }
    out.trim_end().to_string()
}

mod github {
    use super::{Activity, Check, PullRequest, Reviewer, Summary};
    use serde::Deserialize;

    /// `gh pr view --json comments,reviews`.
    #[derive(Deserialize)]
    struct Conversation {
        #[serde(default)]
        comments: Vec<IssueComment>,
        #[serde(default)]
        reviews: Vec<Review>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct IssueComment {
        author: Actor,
        #[serde(default)]
        body: String,
        #[serde(default)]
        created_at: String,
        #[serde(default)]
        url: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Review {
        author: Actor,
        #[serde(default)]
        body: String,
        #[serde(default)]
        state: String,
        #[serde(default)]
        submitted_at: String,
        #[serde(default)]
        url: String,
    }

    /// REST shape from `pulls/N/comments` — snake_case, unlike the `pr view`
    /// JSON above, because it is the raw API rather than gh's own projection.
    #[derive(Deserialize)]
    struct InlineComment {
        user: RestUser,
        #[serde(default)]
        body: String,
        #[serde(default)]
        path: String,
        /// Null once the line it anchored to is gone from the diff; GitHub then
        /// only remembers where it *was*.
        #[serde(default)]
        line: Option<u32>,
        #[serde(default)]
        original_line: Option<u32>,
        #[serde(default)]
        created_at: String,
        #[serde(default)]
        html_url: String,
    }

    #[derive(Deserialize)]
    struct RestUser {
        #[serde(default)]
        login: String,
    }

    pub fn parse_activity(raw: &str) -> Result<Vec<Activity>, String> {
        let parsed: Conversation = serde_json::from_str(raw)
            .map_err(|error| format!("cannot read gh conversation: {error}"))?;
        let mut entries: Vec<Activity> = parsed
            .comments
            .into_iter()
            .map(|comment| Activity {
                kind: "comment",
                author: comment.author.display(),
                body: comment.body,
                created_at: comment.created_at,
                state: String::new(),
                path: String::new(),
                line: 0,
                url: comment.url,
            })
            .collect();
        entries.extend(
            parsed
                .reviews
                .into_iter()
                // Submitting inline comments creates a review row to hang them
                // on, with no verdict and no body. Rendering those would put a
                // blank entry in the thread above the comments they contain.
                .filter(|review| !review.body.trim().is_empty() || review.state != "COMMENTED")
                .map(|review| Activity {
                    kind: "review",
                    author: review.author.display(),
                    body: review.body,
                    created_at: review.submitted_at,
                    state: review.state,
                    path: String::new(),
                    line: 0,
                    url: review.url,
                }),
        );
        Ok(entries)
    }

    pub fn parse_inline_comments(raw: &str) -> Result<Vec<Activity>, String> {
        let parsed: Vec<InlineComment> = serde_json::from_str(raw)
            .map_err(|error| format!("cannot read gh review comments: {error}"))?;
        Ok(parsed
            .into_iter()
            .map(|comment| Activity {
                kind: "inline",
                author: comment.user.login,
                body: comment.body,
                created_at: comment.created_at,
                state: String::new(),
                // Fall back to where it was anchored: an outdated comment still
                // belongs to a file, and dropping to line 0 would say it does not.
                line: comment.line.or(comment.original_line).unwrap_or(0),
                path: comment.path,
                url: comment.html_url,
            })
            .collect())
    }

    #[derive(Deserialize)]
    struct Actor {
        #[serde(default)]
        login: String,
        #[serde(default)]
        name: Option<String>,
    }

    impl Actor {
        /// Prefer the display name, fall back to the handle.
        fn display(&self) -> String {
            match self.name.as_deref() {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => self.login.clone(),
            }
        }
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawPr {
        number: u32,
        #[serde(default)]
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        author: Option<Actor>,
        #[serde(default)]
        base_ref_name: String,
        #[serde(default)]
        head_ref_name: String,
        #[serde(default)]
        base_ref_oid: String,
        #[serde(default)]
        head_ref_oid: String,
        #[serde(default)]
        state: String,
        #[serde(default)]
        is_draft: bool,
        #[serde(default)]
        mergeable: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        additions: u32,
        #[serde(default)]
        deletions: u32,
        #[serde(default)]
        changed_files: u32,
        #[serde(default)]
        review_requests: Vec<RawReviewRequest>,
        #[serde(default)]
        reviews: Vec<RawReview>,
        /// Null when no checks have run, which is why this is an Option and
        /// not a defaulted Vec.
        #[serde(default)]
        status_check_rollup: Option<Vec<RawCheck>>,
    }

    #[derive(Deserialize)]
    struct RawReviewRequest {
        #[serde(default)]
        login: String,
        #[serde(default)]
        name: Option<String>,
    }

    #[derive(Deserialize)]
    struct RawReview {
        #[serde(default)]
        author: Option<Actor>,
        #[serde(default)]
        state: String,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawCheck {
        #[serde(default)]
        name: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        conclusion: String,
        #[serde(default)]
        details_url: String,
    }

    pub fn parse_pull_request(raw: &str) -> Result<PullRequest, String> {
        let pr: RawPr =
            serde_json::from_str(raw).map_err(|error| format!("cannot read gh output: {error}"))?;

        // Requested-but-not-yet-given reviews first, then delivered verdicts.
        // A person who was re-requested after reviewing appears once, as
        // REQUESTED, because that is the state that still needs action.
        let mut reviewers: Vec<Reviewer> = pr
            .review_requests
            .iter()
            .map(|request| Reviewer {
                name: match request.name.as_deref() {
                    Some(name) if !name.is_empty() => name.to_string(),
                    _ => request.login.clone(),
                },
                state: "REQUESTED".into(),
            })
            .collect();
        for review in &pr.reviews {
            let name = review.author.as_ref().map(Actor::display).unwrap_or_default();
            if name.is_empty() || reviewers.iter().any(|existing| existing.name == name) {
                continue;
            }
            reviewers.push(Reviewer { name, state: review.state.clone() });
        }

        Ok(PullRequest {
            number: pr.number,
            title: pr.title,
            body: pr.body,
            author: pr.author.as_ref().map(Actor::display).unwrap_or_default(),
            base: pr.base_ref_name,
            head: pr.head_ref_name,
            base_oid: pr.base_ref_oid,
            head_oid: pr.head_ref_oid,
            state: pr.state,
            draft: pr.is_draft,
            mergeable: pr.mergeable,
            url: pr.url,
            additions: pr.additions,
            deletions: pr.deletions,
            changed_files: pr.changed_files,
            reviewers,
            checks: pr
                .status_check_rollup
                .unwrap_or_default()
                .into_iter()
                .map(|check| Check {
                    name: check.name,
                    status: check.status,
                    conclusion: check.conclusion,
                    url: check.details_url,
                })
                .collect(),
        })
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawSummary {
        number: u32,
        #[serde(default)]
        title: String,
        #[serde(default)]
        author: Option<Actor>,
        #[serde(default)]
        state: String,
        #[serde(default)]
        is_draft: bool,
        #[serde(default)]
        head_ref_name: String,
        #[serde(default)]
        updated_at: String,
    }

    /// `gh repo view --json defaultBranchRef` → `{"defaultBranchRef":{"name":"main"}}`.
    /// The key is null on an empty repository with no commits and so no branches.
    pub fn parse_default_branch(raw: &str) -> Result<String, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Repo {
            default_branch_ref: Option<Named>,
        }
        #[derive(Deserialize)]
        struct Named {
            name: String,
        }
        let repo: Repo =
            serde_json::from_str(raw).map_err(|error| format!("cannot read gh output: {error}"))?;
        repo.default_branch_ref
            .map(|branch| branch.name)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "this repository has no default branch yet".to_string())
    }

    pub fn parse_list(raw: &str) -> Result<Vec<Summary>, String> {
        let rows: Vec<RawSummary> =
            serde_json::from_str(raw).map_err(|error| format!("cannot read gh output: {error}"))?;
        Ok(rows
            .into_iter()
            .map(|row| Summary {
                number: row.number,
                title: row.title,
                author: row.author.as_ref().map(Actor::display).unwrap_or_default(),
                state: row.state,
                draft: row.is_draft,
                head: row.head_ref_name,
                updated_at: row.updated_at,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_forge_from_remote_urls() {
        for url in [
            "git@github.com:valpinkman/jjdiff.git",
            "https://github.com/valpinkman/jjdiff.git",
            "ssh://git@github.com/valpinkman/jjdiff.git",
        ] {
            assert_eq!(Kind::from_remote(url), Some(Kind::GitHub), "{url}");
        }
        // A GitLab remote is now unplaceable, like any other host: jjdiff hides
        // the forge affordances rather than offering ones that cannot work.
        for url in ["git@gitlab.com:group/proj.git", "https://gitlab.example.org/group/proj"] {
            assert_eq!(Kind::from_remote(url), None, "{url}");
        }
        // A host we cannot place must not be guessed at — tangled, sourcehut,
        // Gitea and friends have no CLI jjdiff knows how to drive.
        assert_eq!(Kind::from_remote("git@tangled.org:valpinkman/jjdiff"), None);
        assert_eq!(Kind::from_remote("https://git.sr.ht/~user/repo"), None);
    }

    #[test]
    fn verdicts_parse_the_spellings_the_frontend_sends() {
        // The three literals of `ui/src/ipc.ts`'s Verdict union, verbatim.
        assert_eq!(serde_json::from_str::<Verdict>("\"approve\"").unwrap(), Verdict::Approve);
        assert_eq!(
            serde_json::from_str::<Verdict>("\"requestChanges\"").unwrap(),
            Verdict::RequestChanges
        );
        assert_eq!(serde_json::from_str::<Verdict>("\"comment\"").unwrap(), Verdict::Comment);
    }

    #[test]
    fn head_refs_and_bookmarks_are_namespaced() {
        assert_eq!(Kind::GitHub.head_ref(75), "refs/pull/75/head");
        assert_eq!(Kind::GitHub.local_bookmark(75), "jjdiff-pr-75");
    }

    /// Captured verbatim from `gh repo view --json defaultBranchRef` against
    /// this repository.
    #[test]
    fn reads_the_default_branch_gh_reports() {
        assert_eq!(
            github::parse_default_branch(r#"{"defaultBranchRef":{"name":"main"}}"#).unwrap(),
            "main"
        );
        // A repository with no commits has no default branch. That is a real
        // state, and an error naming it beats opening a proposal against "".
        assert!(github::parse_default_branch(r#"{"defaultBranchRef":null}"#).is_err());
    }

    /// The number is what the banner hangs off, and it is read out of a URL
    /// rather than returned by `gh pr create` in any structured form.
    #[test]
    fn reads_the_proposal_number_out_of_the_url_gh_prints() {
        assert_eq!(
            pull_request_number("https://github.com/valpinkman/jjdiff/pull/12"),
            Some(12),
            "the shape `gh pr create` prints"
        );
        // Not "the last number in the string" and not "the first": an owner or
        // repo may contain digits, and a deep link carries its own.
        assert_eq!(
            pull_request_number("https://github.com/user2/repo90/pull/7/files#discussion_r34"),
            Some(7)
        );
        // Unparseable is `None`, not a panic and not a zero — the caller reports
        // the proposal as open without a number rather than as a failure.
        assert_eq!(pull_request_number("https://github.com/valpinkman/jjdiff"), None);
    }

    /// Captured verbatim from `gh pr view 4 --json …` against this repository,
    /// trimmed to the fields we ask for. Regenerate rather than hand-edit.
    const GH_PR: &str = r#"{
      "additions": 356, "deletions": 6, "changedFiles": 13,
      "author": {"id": "MDQ6VXNlcjY3MTc4Ng==", "is_bot": false, "login": "valpinkman", "name": "Valentin D. Pinkman"},
      "baseRefName": "main", "headRefName": "valpinkman/jjdiff-images-markdown",
      "baseRefOid": "b26222343117d49d65ee7a9222c924c702b7ed64",
      "headRefOid": "70c32eeb155a5142074d34d860691ea9756b4522",
      "body": "Replace two dead ends.", "isDraft": false,
      "mergeable": "UNKNOWN", "number": 4, "state": "MERGED",
      "reviewRequests": [], "reviews": [],
      "statusCheckRollup": [
        {"__typename": "CheckRun", "conclusion": "SUCCESS", "detailsUrl": "https://github.com/x/y/actions/runs/1", "name": "app", "status": "COMPLETED", "workflowName": "App"},
        {"__typename": "CheckRun", "conclusion": "SUCCESS", "detailsUrl": "https://dashboard.gitguardian.com", "name": "GitGuardian Security Checks", "status": "COMPLETED", "workflowName": ""}
      ],
      "title": "C3: Image and markdown rendering",
      "url": "https://github.com/valpinkman/jjdiff/pull/4"
    }"#;

    #[test]
    fn parses_real_gh_pull_request() {
        let pr = github::parse_pull_request(GH_PR).unwrap();
        assert_eq!(pr.number, 4);
        assert_eq!(pr.title, "C3: Image and markdown rendering");
        assert_eq!(pr.author, "Valentin D. Pinkman", "display name wins over login");
        assert_eq!(pr.base, "main");
        assert_eq!(pr.state, "MERGED");
        assert_eq!((pr.additions, pr.deletions, pr.changed_files), (356, 6, 13));
        assert_eq!(pr.checks.len(), 2);
        assert_eq!(pr.checks[0].name, "app");
        assert_eq!(pr.checks[0].conclusion, "SUCCESS");
        assert!(pr.reviewers.is_empty());
        // The OIDs are what makes a *merged* proposal reviewable: this one is
        // MERGED, so `main..head` is empty and only `baseOid..head` shows it.
        assert_eq!(pr.base_oid, "b26222343117d49d65ee7a9222c924c702b7ed64");
        assert_eq!(pr.head_oid, "70c32eeb155a5142074d34d860691ea9756b4522");
    }


    #[test]
    fn reviewers_merge_requests_and_verdicts_without_duplicating() {
        let raw = r#"{
          "number": 9, "reviewRequests": [{"login": "ada", "name": "Ada"}],
          "reviews": [
            {"author": {"login": "grace", "name": "Grace"}, "state": "APPROVED"},
            {"author": {"login": "ada", "name": "Ada"}, "state": "COMMENTED"}
          ],
          "statusCheckRollup": null
        }"#;
        let pr = github::parse_pull_request(raw).unwrap();
        assert_eq!(pr.reviewers.len(), 2, "ada appears once, not twice");
        assert_eq!(pr.reviewers[0].name, "Ada");
        assert_eq!(pr.reviewers[0].state, "REQUESTED", "a re-request outranks an old verdict");
        assert_eq!(pr.reviewers[1].name, "Grace");
        assert_eq!(pr.reviewers[1].state, "APPROVED");
        // A PR with no checks yet must parse, not error.
        assert!(pr.checks.is_empty());
    }

    #[test]
    fn falls_back_to_login_when_the_display_name_is_absent() {
        let raw = r#"{"number": 1, "author": {"login": "octocat"}, "statusCheckRollup": []}"#;
        assert_eq!(github::parse_pull_request(raw).unwrap().author, "octocat");
    }

    // Captured verbatim from `gh pr view 5 --json comments,reviews` on this repo.
    const CONVERSATION: &str = r###"{"comments":[],"reviews":[{"id":"PRR_kwDOTkumRc8AAAABHdJwSA","author":{"login":"valpinkman"},"authorAssociation":"OWNER","body":"## `CLAUDE.md`\n\n**line 1 — you**\n\nNice","submittedAt":"2026-07-28T08:24:07Z","includesCreatedEdit":false,"reactionGroups":[],"state":"COMMENTED","commit":{"oid":"fcd8eafcd540753f11109e2dac0f0766c4c3ac49"}},{"id":"PRR_kwDOTkumRc8AAAABHdNQLA","author":{"login":"valpinkman"},"authorAssociation":"OWNER","body":"","submittedAt":"2026-07-28T08:31:33Z","includesCreatedEdit":false,"reactionGroups":[],"state":"COMMENTED","commit":{"oid":"fcd8eafcd540753f11109e2dac0f0766c4c3ac49"}}]}"###;

    // Captured verbatim from `gh api repos/…/pulls/5/comments`.
    const INLINE: &str = r#"[{"body":"Cool","created_at":"2026-07-28T08:31:33Z","html_url":"https://github.com/valpinkman/jjdiff/pull/5#discussion_r3664008714","line":1,"original_line":1,"path":"CLAUDE.md","user":{"login":"valpinkman"}},{"body":"Outdated one","created_at":"2026-07-28T08:57:17Z","html_url":"https://github.com/valpinkman/jjdiff/pull/5#discussion_r3664163234","line":null,"original_line":42,"path":"CLAUDE.md","user":{"login":"valpinkman"}}]"#;

    #[test]
    fn parses_a_real_conversation_and_drops_empty_review_containers() {
        let entries = github::parse_activity(CONVERSATION).unwrap();
        // Two reviews came back, but the second is the empty container GitHub
        // creates to hang inline comments on — rendering it would put a blank
        // entry in the thread.
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].kind, "review");
        assert_eq!(entries[0].author, "valpinkman");
        assert_eq!(entries[0].state, "COMMENTED");
        assert!(entries[0].body.starts_with("## `CLAUDE.md`"));
        assert_eq!(entries[0].created_at, "2026-07-28T08:24:07Z");
    }

    #[test]
    fn inline_comments_keep_their_anchor_even_when_outdated() {
        let entries = github::parse_inline_comments(INLINE).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.kind == "inline"));
        assert_eq!((entries[0].path.as_str(), entries[0].line), ("CLAUDE.md", 1));
        // `line` is null once the anchor drops out of the diff; falling through
        // to `original_line` keeps the comment attached to a real place.
        assert_eq!(entries[1].line, 42, "outdated comment falls back to original_line");
        assert!(entries[1].url.contains("#discussion_r"));
    }

    #[test]
    fn parses_gh_list() {
        let raw = r#"[{"author": {"login": "valpinkman", "name": "Valentin D. Pinkman"},
          "headRefName": "topic", "isDraft": false, "number": 4, "state": "MERGED",
          "title": "C3", "updatedAt": "2026-07-27T15:53:24Z"}]"#;
        let rows = super::github::parse_list(raw).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].number, 4);
        assert_eq!(rows[0].author, "Valentin D. Pinkman");
    }

    /// `--head` on a branch with no proposal is an ordinary answer, not a
    /// failure: `gh` prints `[]` and exits 0, and the banner is simply absent.
    /// Treating it as an error would put a status message on screen for every
    /// branch that has not been proposed yet, which is most of them.
    #[test]
    fn a_branch_with_no_proposal_parses_to_nothing() {
        assert!(super::github::parse_list("[]").unwrap().into_iter().next().is_none());
    }

    /// `--state all` is what makes a merged proposal findable, so the state
    /// comes through rather than being assumed open — the banner reads
    /// "merged", and this is the field it reads.
    #[test]
    fn a_merged_proposal_keeps_its_state() {
        let raw = r#"[{"author": {"login": "valpinkman", "name": "Valentin D. Pinkman"},
          "headRefName": "valpinkman/topic", "isDraft": false, "number": 4,
          "state": "MERGED", "title": "C3", "updatedAt": "2026-07-27T15:53:24Z"}]"#;
        let found = super::github::parse_list(raw).unwrap().into_iter().next().unwrap();
        assert_eq!((found.number, found.state.as_str()), (4, "MERGED"));
        assert_eq!(found.head, "valpinkman/topic");
    }

    fn comment(path: &str, line: u32, side: &str, body: &str) -> ReviewComment {
        ReviewComment {
            path: path.into(),
            line,
            side: side.into(),
            body: body.into(),
        }
    }

    #[test]
    fn diff_sides_map_to_the_forge_vocabulary() {
        assert_eq!(comment("a.rs", 1, "new", "x").github_side(), "RIGHT");
        assert_eq!(comment("a.rs", 1, "old", "x").github_side(), "LEFT");
    }

    #[test]
    fn folding_comments_into_the_body_loses_nothing() {
        // The fallback when the forge rejects inline anchoring. Every comment
        // must still reach the reviewer, with enough to locate it by hand.
        let merged = merge_comments_into_body(
            "Overall looks good.",
            &[
                comment("src/sync/engine.ts", 14, "new", "This can throw."),
                comment("src/old.rs", 3, "old", "Why was this dropped?"),
            ],
        );
        assert!(merged.starts_with("Overall looks good."));
        assert!(merged.contains("`src/sync/engine.ts`"));
        assert!(merged.contains("line 14 (after)"));
        assert!(merged.contains("This can throw."));
        assert!(merged.contains("`src/old.rs`"));
        assert!(merged.contains("line 3 (before)"));
        assert!(merged.contains("Why was this dropped?"));
    }

    #[test]
    fn folding_handles_an_empty_body_and_no_comments() {
        assert_eq!(merge_comments_into_body("just a note", &[]), "just a note");
        let only_comments = merge_comments_into_body("", &[comment("a.rs", 1, "new", "hi")]);
        assert!(only_comments.starts_with("**`a.rs`**"), "no leading blank lines: {only_comments:?}");
    }

    #[test]
    fn first_line_summarises_a_multiline_forge_error() {
        let stderr = "\ngh: Unprocessable Entity (HTTP 422)\nline must be part of the diff\n";
        assert_eq!(first_line(stderr), "gh: Unprocessable Entity (HTTP 422)");
    }

}
