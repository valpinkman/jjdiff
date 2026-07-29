# jjdiff — plan

**jjdiff**: a fast, minimal desktop diff viewer for reviewing and landing changes in **Jujutsu
colocated repos**, built with **Tauri 2 + Rust + Lit**, jj-native from day one. Informed by a code-level analysis of [nkzw-tech/codiff](https://github.com/nkzw-tech/codiff)
(MIT, Electron): its expensive part is the diff-review UI (~38k LOC renderer); its VCS plumbing is
~3k LOC and cleanly separated behind an IPC surface. We copy the *shape*, not the code — and we
design the review model around jj instead of bending git's index model.

## Product theses

1. **No staging axis.** git tools revolve around staged/unstaged. jj has no index — the working
   copy *is* a commit (`@`). The core loop becomes: edit → review `@` → `describe` → `new`.
   Partial commits become `squash`/`split`, not `git add -p`.
2. **Review the stack, not just a diff.** jj users work in stacks of small changes. First-class
   UI: a stack sidebar (`trunk()..@`), per-change diffs, and "move this file/hunk into that
   change" (`jj squash --into`), plus one-click `jj absorb`.
3. **Change IDs are the identity.** Review state (viewed flags, comments) keys on stable change
   ids, not commit hashes — it survives `describe`, `squash`, rebases. This is impossible to do
   well in git and trivial in jj; it's the killer feature.
4. **Conflicts are data, not errors.** jj materializes conflicts inside commits. Render them as a
   structured three-pane/annotated view instead of raw markers.

## Architecture

```
┌─────────────────────────────────────────────┐
│ ui/            Lit 3 + Vite + TS            │
│   diff view (virtualized), stack sidebar,   │
│   file tree, command bar, comments          │
├──────────────── Tauri IPC ──────────────────┤
│ src-tauri/     app shell, windows, menu,    │
│                CLI entry (`jjdiff [revset]`)│
│ crates/vcs     jj facade: repo model,       │
│                revsets, statuses, mutations │
│ crates/diff    blob loading + diffing       │
│                (imara-diff, word-level)     │
│ crates/watch   op-log + fs watcher (notify) │
└─────────────────────────────────────────────┘
```

- **Tauri 2.x** — ~10 MB bundle / low RSS vs Electron's ~200 MB; native menus; sidecar-free
  because the backend *is* Rust.
- **Frontend: Lit 3 + Vite + TypeScript.** ~5 KB runtime, no VDOM — the frontend counterpart of
  choosing Tauri over Electron. The diff view is custom DOM-heavy work in any framework; Lit's
  fine-grained templating (`repeat()`, no reconciler) suits huge code listings. Virtualization
  via `@lit-labs/virtualizer` (variable-height, fits diff hunks); state via reactive
  controllers / `@lit-labs/signals`; syntax highlighting with **Shiki in a web worker** (lazy
  grammars, zero Rust-side grammar management). Revisit syntect/tree-sitter in Rust only if
  profiling demands it.
  - **Light-DOM rule:** the code/diff pane renders in light DOM
    (`createRenderRoot() { return this }`) so text selection, copy, and find work natively
    across lines; shadow DOM is reserved for chrome (sidebar, dialogs, command bar). Theming
    flows exclusively through CSS custom properties so it crosses both worlds.
- **Rust workspace** (`crates/`) so the vcs/diff/watch logic is testable without Tauri.

### jj integration strategy: CLI-first, jj-lib later

jj is itself Rust, so linking `jj-lib` is tempting, but its API is unstable and version-coupled
to the user's repo. Phase 1 is **CLI-first**:

- All reads run `jj … --ignore-working-copy --color never` — never triggers snapshots, never
  contends for the working-copy lock, never pollutes the op log.
- Structured output via `-T` templates wherever supported (`log`, `file show`, `op log`) —
  jj templates are a real query language; design one template module with escaping helpers.
- Mutations go through the CLI (`describe`, `new`, `squash`, `split`, `absorb`, `restore`) so
  semantics always match what the user's own `jj` does, across jj versions.
- **Blob fast path:** colocated repos have a real `.git`, so plain file contents are read by oid
  via **gix** (no process spawn, batch-friendly — replaces git's `cat-file --batch` trick).
  ⚠ Caveat: jj stores *conflicted* trees using special `.jjconflict-*` entries in the git
  backend; gix tree-walking must detect these and fall back to `jj file show` / jj's resolver.
- Phase 2 (optional, perf-driven): adopt `jj-lib` for hot read paths (tree diffs, evolog,
  conflict model) behind the same `crates/vcs` trait, pinned per release.

### Command / capability mapping

| Need | Source |
|---|---|
| repo root, is-jj-repo | `jj root` |
| working-copy live diff | fs contents vs `@-` tree (gix + crates/diff) — **not** jj snapshot, so the view is live without writing ops |
| diff of a revset / range | `jj diff --git -r X` for statuses/renames, blobs via gix, diff in Rust |
| file statuses | `jj diff --summary` (fallback) → prefer `-T` templates / tree diff |
| history / stack | `jj log --no-graph -T <json-ish template> -r 'trunk()..@'` |
| merge base | revset `fork_point(A \| B)` |
| identity | `jj config get user.name/email` |
| commit flow | `describe -m` / `new`; partial: `squash --into X -- paths`, `split` |
| auto-distribute hunks | `jj absorb` |
| rewrite tracking (review-state migration) | `jj evolog -T` |
| change detection | watch `.jj/repo/op_heads/heads/` + debounced fs events (notify crate) |

Hunk-level (not file-level) squash/split is the one thing the CLI can't do
non-interactively. Phase 1 ships file-level; the scripted diff-editor shim this predicted
is what hunk-level split was eventually built on — jjdiff registers itself as jj's diff
editor for one `jj split` and edits the directory jj hands it. `squash -i` speaks the same
protocol and is the remaining half.

### Review-state model

- SQLite (rusqlite) in app data dir, keyed `(repo_id, change_id)`: viewed flags, draft comments,
  walkthrough progress. On op-log change, reconcile via `evolog` so state follows rewrites;
  orphan state on abandoned changes is garbage-collected.
- Config: `~/.config/jjdiff/config.toml` (+ JSON schema for the UI editor), watched live.

### IPC surface (Tauri commands — mirrors codiff's proven verbs)

`get_repo_state(source)`, `get_stack()`, `get_diff(change_id|range, path?)`,
`get_file_contents(rev, path)`, `describe(change_id, msg)`, `new_change()`,
`squash(paths, from, into)`, `absorb()`, `set_viewed(change_id, path, bool)`,
`get_conflicts(change_id)` — plus a `repo-changed` event stream from the watcher.

## Milestones

**M0 — skeleton. ✅ DONE.** Tauri scaffold, Rust workspace, `jj` CLI wrapper with
`--ignore-working-copy` discipline, op-head watcher, `jjdiff [revset]` CLI entry.

**M1 — read-only reviewer. ✅ DONE.** Working-copy (fs-vs-`@-` via gix, zero snapshots —
verified by op-log test) + arbitrary-revset diffs; virtualized split/unified views, word-level
intra-line spans (UTF-16 offsets), whitespace toggle; file tree; shiki/core highlighting in a
worker (JS regex engine, only mapped grammars bundled); binary/too-large guards.

**M2 — jj-native actions. ✅ DONE.** Describe editor + Commit & New (`describe` + `new`),
file-level `squash` to parent, viewed flags keyed by change id (collapse + tree dimming),
Cmd/Ctrl+Shift+P command bar, `~/.config/jjdiff/config.toml`, working-copy fs watcher
(gitignore-aware) so edits appear without a manual refresh.

**M3 — stack review. ✅ DONE.** Stack sidebar over `trunk()..@`, per-change review with
Mark Reviewed (stores the reviewed commit id per change id), "what changed since I last
reviewed" interdiff via `jj interdiff` between evolog commits, Absorb button (summary
surfaced), per-file move-to-change over mutable stack changes.
Note: evolog's template context differs from log's (`json(commit)`, not `json(self)`).

**M4 — conflicts + polish. ✅ DONE.** Conflict surfacing (banner, per-file badges,
highlighted jj conflict-marker lines in diffs, `resolve --list` parsing), forced
light/dark/system theme, configurable command-bar keymap, repo name in the window title,
packaging config (app/dmg/deb/rpm targets, generated icon set).
Deliberate cut: no in-app `jj resolve` launcher — interactive merge editors spawned from a
GUI without a TTY hang more often than they work; the conflict banner points at the
terminal instead.

**M5 — guided walkthroughs (Claude backend). ✅ DONE.** `jjdiff -w` and in-app generation:
structured diff (stable hunk ids) → prompt with guide + JSON contract → Claude Code CLI
headless (`claude -p --output-format json`, stdin prompt, timeout, hallucinated-id
filtering) → stored per **change id** with a diff fingerprint, so an evolved change flags
the walkthrough stale and offers regeneration. Guided mode: overview + steps, each step
filters the diff to its hunks; ←/→ navigation. Backend sits behind an `AgentBackend` trait —
Claude only for now, more CLIs later. Verified end-to-end against the real CLI
(`cargo test -p jjdiff-app real_claude -- --ignored`).

**M6 — daily-driver polish. ✅ DONE.** Keyboard-first review (`j`/`k` files, `n`/`p`
hunks, `v` viewed) with a virtualizer-driven cursor; find-in-diffs (`Mod+F`, live match
count, wrap-around); tree-click scrolls instead of filtering plus a sticky file
breadcrumb; expand-context (`file_content` command; expanders derived from hunk gaps so
they appear before the file is fetched); word-wrap toggle; `jjdiff <revset>` selects a
change on launch.

**M7 — agent + robustness. ✅ DONE.** All four agent CLIs behind one `CliBackend`
(Claude/Codex/OpenCode/Pi — one spawn path, per-backend argv, envelope extraction covering
single-object, JSONL-stream and nested-message shapes), selected via `[walkthrough]
backend`. Agent-authored walkthroughs: `jjdiff --print-hunks` dumps the diff with stable
ids headlessly, `--walkthrough-file` imports the agent's JSON through the same validation
as a generated one, and `skills/jjdiff/SKILL.md` documents the loop. `check_version()`
gates on jj ≥ 0.33. Worktree rename detection (exact-content only) and nested `.gitignore`
support in the fs watcher close the two known correctness gaps.

## Phase 2 — jj as a whole tool ✅ SHIPPED

M0–M7 made jjdiff a *reviewer*. Phase 2 makes it a *client*: acting on the repo, not just
reading it. Ordering is deliberate — the safety net lands before the sharp tools.

### B1 — Bug: description hidden on non-stack changes ✅ FIXED

Selecting an older change shows an empty description box. `select()` looks the change up in
`repo.stack` only ([app.ts:404](ui/src/app.ts:404)), but the stack is `trunk()..@ | @`, so
anything below trunk resolves to `undefined` and seeds `''`. It also sets `seededFor`, which
suppresses the correcting re-seed in `refresh()`. Fix: resolve through `selectedChange`
(which already searches the graph) and render immutable descriptions read-only rather than
as an editable box that silently discards edits.

**Superseded in part.** Immutable descriptions are editable again — but the premise has
changed, not been ignored. Read-only was right when the alternative was an edit that went
nowhere; now Save routes through the immutable-rewrite confirmation and actually lands, so
the box no longer lies about what it does.

### M8 — Change detail view ✅

Clicking a change currently jumps to Files, which throws away the change's identity. Replace
that with a **detail pane** as the default view for any non-working-copy change:

- Identity: change id, commit id, author, timestamps, bookmarks, immutable/conflict/empty
  state, and the full multi-line description (read-only when immutable, editable when not).
- Changed-file list with +/− counts, click to jump into the diff.
- Parents/children as navigable links — walking the graph without the sidebar.
- Actions relevant to *that* change (see M10), disabled with a reason when not applicable.
- The working copy keeps today's edit-first layout; the detail view is for everything else.

### M9 — Operation log + undo ✅

jj's superpower is that every operation is reversible. Shipping this **before** the mutation
surface means every new command in M10 arrives with a safety net.

- **Op log tab**: `jj op log -T 'json(self)'` returns id, parents, start/end time,
  description, and the literal `args` — a structured log with no parsing at all.
- **Undo**: `jj undo` for the last operation; `jj op restore <id>` to jump back to any point;
  `jj op revert <id>` to reverse a specific one mid-history.
- **Undo affordance on every mutation**: each command reports jj's own summary plus an Undo
  action, so experimenting is cheap. This is the feature that makes the rest safe.
- **`jj op diff`** between two operations for "what did that actually do".

### M10 — The jj command surface ✅

One `mutate()` helper in `crates/vcs`: run, capture stdout+stderr, return the resulting
operation id for undo. Every command below routes through it, appears in the command bar with
a keybinding, and is disabled with an explanation when it cannot apply (immutable target,
no remote, nothing to squash).

| Group | Commands | Notes |
|---|---|---|
| Navigate | `new [rev]`, `edit <rev>` | "Work on this change" from the detail view |
| Shape | `squash` (whole + per file ✅), `absorb` ✅, `split <paths>` ✅ + per hunk ✅, `duplicate`, `abandon` | Both splits ship: file-level is `jj split <paths>`, hunk-level drives jj's diff editor (see *Still open after Phase 2*). Hunk-level `squash` is the piece still missing |
| History | `rebase -r/-s/-b -d <dest>`, `backout` | Destination picker over the graph; drag-to-rebase deliberately not first — misdrops are expensive, and every rebase is undoable but not free |
| Describe | `describe` ✅, bulk-describe empty changes | |
| Remote | `git fetch`, `git push -b/--change`, bookmark create/set/delete/track, **open pull request** | Push needs bookmarks; `--change` auto-names from the change id. Show ahead/behind per bookmark. See PR note below |
| Files | `restore <paths>` | Discard changes — the one genuinely destructive op; confirm, and it is undoable |

**Safety model** (applies to all of the above): destructive commands (`abandon`, `restore`,
`op abandon`) confirm first;
everything reports what it did and offers Undo. Long operations (fetch/push) run async with
progress, since they already block on the network.

**Immutable targets are warned about, not blocked.** They used to be disabled outright,
which is the safe default and the wrong one for the case that actually arises: fixing your
own already-pushed commit. Now `describe`, `edit`, `abandon`, `rebase` and `split` are
offered on immutable changes and gated by a dialog naming the bookmark, the force-push it
implies, and the descendants that get rebased — after which jjdiff passes
`--ignore-immutable` for that one command. It is a per-call argument end to end, never a
mode or a setting: jj's guarantee is worth keeping for the next command as well as this one.

### M11 — Revset filtering ✅

The graph is hardwired to `ancestors(@ | bookmarks())` capped at 60. Replace with a filter
control:

- A preset row for the revsets people actually use: **Stack** (`trunk()..@`), **All**
  (`ancestors(@ | bookmarks())`), **Mine** (`author(exact:"<me>")`), **Recent**
  (`ancestors(@, 50)`), **Conflicts** (`conflicts()`), **Bookmarks** (`bookmarks()`),
  **Working copies** (`@ | @-`).
- A free-form revset input with jj's own error surfaced inline on a bad expression (jj
  validates far better than we could).
- Persisted per repo, and honoured by `jjdiff <revset>` on launch.

#### Pull requests without a forge integration

Every forge prints a ready-made "create a PR" URL on the push that creates a branch —
tangled (`…/pulls/new?sourceBranch=…&targetBranch=main`), GitHub
(`…/pull/new/<branch>`), GitLab (`…/merge_requests/new?…`). We already capture push
stderr for the mutation summary, so the plan is to **scrape that URL and offer an "Open
pull request" action** rather than integrate any forge API.

That buys tangled, GitHub, GitLab and Gitea support in one small feature with no auth, no
tokens, and nothing to keep current as APIs drift. A real API integration (PR title/body
from the change description, review state in-app) stays deferred until there is a reason to
pick one forge — and tangled would now be the natural first, not GitHub.

Fetch pairs with this: `jj git fetch` plus per-bookmark ahead/behind is what tells you a PR
is out of date before you push over it.

### Still open after Phase 2

- ~~**Interactive (hunk-level) split**~~ ✅ **DONE.** The scripted diff-editor shim this
  entry always called for, with jjdiff as the shim: `jj split -i` hands two directories
  to a diff editor and takes whatever the right one holds, so jjdiff registers *itself*
  as that editor for one invocation and jj re-enters the binary as
  `--apply-split-plan`. The plan is built from the diff on screen rather than
  recomputed, which is the point — the hunks that move are the ones that were ticked.
  What makes that safe to do to someone's commit is a check rather than trust:
  applying **every** hunk must reproduce the new side exactly, a property of any correct
  diff, so it is free when things are well and refuses a stale plan, a file edited since
  it was read, or a diff of some other pair of trees. It also means our hunk boundaries
  need not agree with jj's, since any correct cut of the same change composes back to the
  same result. A refusal exits non-zero and jj abandons the split whole.
- ~~**Ahead/behind per bookmark**~~ ✅ **DONE.** `Repo::bookmark_statuses` templates
  `jj bookmark list --all-remotes` for `tracking_ahead_count`/`tracking_behind_count`,
  which are stated from the *remote* ref's side and so mean the opposite of the names
  jjdiff uses — the inversion happens once, in that function, and is tested in both
  directions against a real remote. Bookmark tags show `↑2 ↓1` (neutral: a position is
  not an outcome) and disappear when in sync. The payoff is on the forge banner: an
  unpushed head means CI, reviewers and merge state describe code the forge has and the
  reviewer does not, so `renderHeadDrift` says so next to the checks.
- ~~**Rebase destination picker**~~ ✅ **DONE.** The revset prompt asked the wrong
  question: the destination is nearly always a commit already on screen, so naming it
  meant reading an id off the graph and retyping it — a transcription step whose only
  possible contribution is an error. The picker lists the graph, filterable, with
  bookmarks and immutability visible, and the mode (`-r` / `-s` / `-b`) is a labelled
  choice rather than a hardcoded `-s`. Destinations that would form a cycle — the change
  and its descendants — are not offered at all, so jj's refusal arrives before the
  confirmation instead of after it. The free-form field stays, below the list: `trunk()`
  and `main@origin` are real answers that no list of commits contains. Drag-to-rebase is
  still deliberately not first; misdrops are undoable but not free.
- **Conflict resolution** — resolving is still terminal-only, for the reason given in
  M4, but the conflict is no longer a dead end. `jj resolve --list` gives every path
  *and* jj's description of each ("2-sided conflict"), which is the only statement of a
  conflict's arity outside the marker lines; the banner names them, clicking one jumps
  to it, and `c`/`C` step between conflict **regions** rather than files, since one file
  routinely holds several. The markers themselves are coloured by role — fence, side,
  base — because jj, unlike git, already writes each side's commit and description into
  the marker text, so the colour only has to say which kind of side you are looking at.
  What remains deferred is the merge editor itself.
- ~~**`jj op diff`**~~ ✅ **DONE.** Every row in the operation log answers "what did that
  actually do": *What changed* narrates one operation against its parent, and pinning a row
  as *Compare from here* narrates a span of them — which is the case that matters, since
  what usually needs explaining is the last several operations rather than any one. This is
  the single read that returns text instead of a structure, deliberately: `jj op diff` takes
  no `-T` and has no `json()` form, so jjdiff displays its narration verbatim rather than
  parsing prose, which is what the templates invariant exists to prevent.
- ~~**Evolog drawer**~~ ✅ **DONE.** Every version a change has been — `jj evolog` — with an
  interdiff between any two of them, reached from the change's overflow menu or the palette.
  The A/B column selection is the wiki page-history idiom: A is the older side, and the
  radios that would invert the direction are disabled rather than silently reordered. jj
  excludes rebase noise, so the result is how the *change* differs, not how the commits do.
  Two versions of one change is a comparison git cannot express at all — a rewritten commit
  is garbage the moment nothing points at it.

### Proactive additions not in the original ask

- **Undo everywhere** (M9) — the single highest-value item here, and the reason to do the op
  log before the commands.
- **Bookmark management** (M10) — not optional: `jj git push` has nothing to push without it.
- **Fetch + ahead/behind** — a review tool that cannot see the remote's state is half-blind.
- **PR creation by URL scraping** — forge-agnostic, no API integration (see above).
- **Evolog drawer** ✅ — we already fetched evolog for interdiffs; exposing "this change has
  6 versions, diff any two" was nearly free and has no git equivalent. Shipped; see *Still
  open after Phase 2* above.
- **Conflict resolution** — still deferred (GUI-spawned merge editors without a TTY hang more
  than they work), but M10 should at least offer `jj resolve --list` navigation and mark
  which side is which.
- **Command discoverability** — every command in the command bar with a keybinding, so the
  UI does not become a hunt for buttons.

### Phase 3 — collaboration and reach (planned)

Phase 1 made jjdiff a reviewer; Phase 2 made it a jj client. What remains is everything
that involves *other people* — comments, forges, sharing — plus the terminal entry point
that makes it reachable at all. Ordered by value per unit of work.

### C1 — CLI and terminal helper ✅ DONE

**We have a CLI.** `Args::parse` (in `src-tauri/src/cli.rs`) runs at the very top of
`main()`, before `tauri::Builder`; `--help`, `--version`, `--walkthrough-guide`,
`--diff`, `--print-hunks` and `--install-terminal-helper` write to stdout and `exit(0)`
without ever creating a window. A bundled macOS binary still has a usable stdout when
invoked from a terminal, so this works without a Node shim.

**We have no CLI.** `LaunchOptions::from_env` parses `-R`, `-w`, `--walkthrough-file` and a
positional revset, but only inside the app binary: there is no `jjdiff` on `PATH`, no
`--help`, and no way to launch from a shell without knowing the bundle path. Codiff solves
this with `bin/codiff.js` plus an "Install Terminal Helper" menu item.

Tauri makes this different from codiff's Electron approach — there is no Node shim, the
app binary *is* the CLI:

1. **Headless-before-GUI.** Parse argv at the very top of `run()`, before
   `tauri::Builder`. `--help`, `--version`, `--walkthrough-guide` and `--print-diff` write
   to stdout and `exit(0)` without ever creating a window. A bundled macOS binary still
   has a usable stdout when invoked from a terminal, so this works.
2. **A shim on PATH.** `Install Terminal Helper` (menu + command bar) writes a two-line
   `exec` script pointing at the bundle binary. Prefer `~/.local/bin`, fall back to
   `/usr/local/bin`, and *never* silently sudo — if neither is writable, print the one
   command the user should run.
3. **Single instance with arg forwarding** (`tauri-plugin-single-instance`): running
   `jjdiff` in a second repo opens a second window in the existing process rather than a
   rival process fighting over the same review store.

Command surface, mirroring what the app already does:

```
jjdiff [revset]              # open a repo (defaults to cwd)
jjdiff -R <path> [revset]    # explicit repo
jjdiff -w [revset]           # open and generate a walkthrough
jjdiff --walkthrough-file f  # open an agent-authored walkthrough
jjdiff --walkthrough-guide   # print the authoring guide (headless; for agents)
jjdiff --diff [revset]       # print the structured diff as JSON (headless; scripting)
jjdiff --help / --version
```

The `--walkthrough-guide` + `--walkthrough-file` pair closes the agent loop we already
built: a skill can ask jjdiff for its guide, author a walkthrough, and open jjdiff on it —
codiff's `$codiff` pattern, which currently has no entry point on our side.

### C2 — Inline review comments ✅ DONE

The single biggest gap, and the place where jj lets us beat codiff rather than match it.

- **Anchor on change ids, not commits.** A comment records `(change id, path, hunk id,
  side, line)`. Because change ids survive `describe`/`squash`/rebase, a comment stays
  attached to the code it was about — structurally impossible in git, where codiff must
  re-anchor to a commit sha.
- **Drift handling.** Store the commit id the comment was written against. When the change
  evolves, re-anchor by matching line content within the file; if the line is gone, mark
  the comment **outdated** and show it against its original text rather than silently
  dropping or misplacing it.
- **Storage moves to SQLite** (rusqlite). The JSON review store was right for flags and
  walkthroughs; threaded comments with anchors and timestamps want real queries.
- **UI**: click a line number to open an inline composer; comments render under their line;
  a Review tab lists every pending comment with its file and status; **Copy as Markdown**
  produces a paste-ready review for a chat or an agent prompt.

Ships useful without any forge integration — copy-as-markdown alone covers the solo and
agent-assisted workflows.

### C3 — Content jjdiff currently refuses to show ✅ DONE

Two dead ends where we print a shrug:

- **Images.** `Binary file` today. Needs a `file_bytes(revset, path)` command returning
  base64 + mime, and a side-by-side old/new image view with a size cap. Renames and
  dimension changes are the interesting cases.
- **Markdown.** Rendered preview for `.md` files and for walkthrough/plan documents.
  Diffs of markdown stay diffs — this is a *view* toggle, not a replacement.

Small, self-contained, and each one removes a visible "the tool can't do this".

### C4 — Forge review via `gh` ✅ DONE

Reviewing *other people's* work, which jjdiff cannot do at all today. Scoped to the CLIs
rather than REST APIs, so auth is someone else's problem:

- `jjdiff pr 75` / `jjdiff mr 23` ✅ — fetch the head, review it as a normal diff.
- Show reviewers, merge state and CI checks alongside the diff ✅ — a banner above the diff.
- Submit a review (approve / request changes / comment) from the accumulated C2 comments ✅.
- Colocated repos make this natural: the PR branch is a git ref jj can already address ✅.

**The revset is the interesting part.** The obvious `base..head` is wrong, and silently so:
the moment a proposal merges, its head becomes an ancestor of the base branch and the
revset goes *empty* — the review shows nothing, with no error. The forge already knows the
right answer, so we ask it: `gh` exposes `baseRefOid` (GitLab: `diff_refs.base_sha`), which
is the merge base it diffs against, and `baseRefOid..head` is correct for open and merged
proposals alike. `open_pull_request` also fetches the base branch, because that OID has to
exist locally for the revset to resolve.

Everything downstream is unchanged: the head lands on a namespaced bookmark
(`jjdiff-pr-75`), so from there a proposal is an ordinary revset reviewed by the same diff
pane, walkthroughs and inline comments as anything else.

**A proposal is context, not a mode.** The first cut made reviewing a PR a separate view you
entered by number and left by closing. That was backwards: `gh pr list` already reports each
proposal's head branch, and every change already carries its bookmarks, so a match is enough
to show the banner *on the change you are looking at*. Working on your own branch now surfaces
its CI, reviewers and merge state without being asked. `jjdiff pr N` remains for the case the
match cannot cover — someone else's branch, which is not local until it is fetched — and once
fetched it resolves through the same match. Diffing the whole proposal rather than the selected
commit is a toggle on the banner.

Inline comments post as **real line comments** via `gh api …/pulls/N/reviews`, which is what
anchoring on change ids was for. That endpoint is all-or-nothing — one comment on a line
outside the diff rejects the entire review — so outdated comments are filtered first and a
rejection retries with everything folded into the body rather than losing the reviewer's work.

Forge detection is inferred from the remote URL rather than configured. A host we cannot
place is an error, not a guess, and the Forge command group is absent entirely on a repo
we cannot drive — an affordance that always fails is worse than one that is not there.

**The conversation is part of the review.** The banner carries the proposal's
description and its full thread — discussion comments, review verdicts and
line-anchored comments — merged from the three places GitHub keeps them and
sorted by time. It is height-capped and scrolled internally rather than left to
grow: the diff is the content, and a long description with a dozen comments
would otherwise push it off screen on every selection. Forge markdown is
sanitised through an allow-list before rendering; it is untrusted text in a
WebView holding the whole IPC surface.

**GitLab was dropped.** It was written against `glab`'s documented JSON and
never run against a live instance, which is worse than absent — it advertised
support that had never worked once. The code is in history if it is ever worth
finishing against a real instance.

### C5 — Native app polish ✅ DONE

Individually small, collectively the difference between "a window" and "a Mac app":

- **Menu bar** ✅ (`src-tauri/src/menu.rs`). Built *from* the palette rather than alongside
  it: `app.ts` pushes its live command list through `set_menu`, each item carries a command
  id, and a click emits `menu-command` for the frontend to run. There is no second
  definition to drift. The app/File/Edit/Window menus come from Tauri's predefined items —
  Edit is load-bearing, since a custom menu without it costs the WebView Cmd+C/Cmd+V.
  Mirrored items deliberately carry no accelerators: the frontend already dispatches every
  shortcut, and a menu accelerator would shadow or double-fire it.
- **Keyboard shortcuts help** ✅ — `?` opens `ui/src/shortcuts-help.ts`, driven by
  `shortcutReference()` in `keys.ts`. `formatShortcut` renders bindings per platform
  (⌘K / Ctrl+K) and the palette hints use it too, so the two agree.
- **Multi-window** ✅ — one window per repo. This was the real work: `AppState` used to hold
  a single `Repo`, so repo state is now per-window (`WindowState`, keyed by window label)
  and all 37 repo-touching commands resolve through `repo_handle(state, window)`.
  `repo-changed` is emitted per repo root, not app-wide. A second `jjdiff` invocation
  focuses the window already showing that repo, or opens a new one. Capabilities had to
  widen to `repo-*` or new windows would have had no IPC at all.
- **Open in editor** ✅ — `[editor] command` with `{file}`, `{line}`, `{repo}`. Bound to `o`
  (uses the diff cursor, so it opens at the line under review) and the file-tree context
  menu. Templates split before substitution, so a path with spaces stays one argument and
  a filename cannot inject a flag; no shell is involved.
- **App icon** ✅ — `src-tauri/icons/icon.svg` is the source; regenerate with `rsvg-convert`
  + `pnpm tauri icon`. Split-diff mark in the DESIGN.md dark-theme diff colours.

Still open from the original list: **drag-to-rebase** was never part of C5, and per-window
menu accelerators were skipped on purpose (see above).

### C6 — Shared review links (deliberately last)

Codiff's Cloudflare service turns a walkthrough into a URL. It is the largest item here
(a service, a database, auth, retention) and the least useful while jjdiff has one user.
Revisit only if people other than the author start reviewing with it.

### D1 — Design system, named themes, app shell ✅ DONE

Not in the original plan, and the largest single change since C4: the frontend had grown
feature-first for seven milestones and looked it. Presentation and layout only — no IPC, no
jj semantics; the two Rust changes are a config writer and the window chrome. The binding
spec is [DESIGN.md](DESIGN.md), rewritten to match.

- **A token layer.** Four named ramps (space, type, radius, elevation) and three motion
  curves in `theme.css`. Nothing outside them, so a padding a pixel off is a bug rather than
  a judgement call.
- **Neutrals rebased on a zero-chroma grey ramp.** The old set was warm, and three
  off-whites within 3% of each other made page → panel → card invisible. Grey also gets out
  of the way of the green and red, which are the only colours here that mean anything — the
  same reasoning makes the primary action neutral, so the accent goes back to meaning one
  thing.
- **Radius is binary: surfaces are square, controls are pills.** The middle ground was three
  nested frames at every point on screen, none of which meant anything.
- **Nineteen named palettes** (Catppuccin, Rosé Pine, Ayu, Nord, Tokyo Night, Gruvbox,
  Everforest, Solarized, Dracula, One Dark, Kanagawa) in `ui/src/themes.ts`, each **derived**
  from about a dozen seed colours rather than written out — hand-writing nineteen token sets
  guarantees the twentieth token is defined in three of them. Every seed names a shiki theme
  loaded on demand, because Nord chrome around GitHub-coloured code is the one thing that
  would make the feature look fake. Chosen from a swatch picker with live hover preview;
  persisted through a new `set_ui_theme`.
- **An icon rail replaces the sidebar tabs**, which never fit four labels and two badges at a
  readable size; the sidebar folds away entirely (⌘B) for review and guided steps.
- **Every pane is a card**, including the diff, and the current file's header pins while you
  are inside it so `viewed` stays reachable. `position: sticky` cannot work there — the
  virtualizer positions rows absolutely — so it overlays from outside the scroller.
- **Native title bar is an Overlay** with the title hidden and the header as the drag region.

Six latent bugs surfaced on the way, the two worth naming being `visibilityChanged` carrying
`first`/`last` as own properties rather than on `detail` (so the breadcrumb reported the
first file in the diff no matter where you had scrolled), and a `1fr` grid track's automatic
minimum being `min-content` (so the diff pushed the shell wider than the window and the whole
app scrolled sideways, carrying the toolbar off screen).

### Suggested order

1. **C1** ✅ — nothing else is reachable from a terminal without it, and it is a day or two.
2. **C2** ✅ — the biggest capability gap; ~1.5–2 weeks.
3. **C3** ✅ — a few days, removes two dead ends.
4. **C5** ✅ — polish, can be interleaved whenever.
5. **C4** ✅ — only when reviewing others' PRs actually matters to you.
6. **D1** ✅ — unplanned, and taken once the surface stopped moving.
7. **C6** — probably never, and that is fine.

## Risks specific to Phase 2

- **Mutations move the working copy.** `jj edit`/`new`/`rebase` rewrite files on disk; the
  watchers already catch this, but the UI must not hold stale paths across the change.
- **Rebase produces conflicts by design in jj** — that is not an error path, and the UI must
  present it as a normal outcome with the conflict surfacing we already have.
- **`jj op restore` rewrites the working copy too** — the same refresh discipline applies,
  and it must never be offered without a confirmation naming what it will undo.
- **Command sprawl.** Every command needs an applicability rule, a confirmation policy, and a
  test; the `mutate()` helper exists to keep that uniform rather than ad hoc per command.

**Deferred, with reasons.**
- *Signed macOS build + Homebrew tap* — blocked on an Apple Developer identity; the
  unsigned bundle already builds (`pnpm tauri build`).
- *Shared-review web service* — weeks of work (codiff's Cloudflare equivalent) and
  questionable value before there is a second user.
- *Hunk-level **squash*** — the split shipped (see above) and `jj squash -i` speaks the
  same diff-editor protocol, so the shim is already written; what is missing is only the
  plan-building for a two-change selection. File-level `squash`/`absorb` covers most of
  the need today.
- *Full-file syntax highlighting* — expand-context lines are deliberately untokenized;
  proper highlighting wants the whole file through shiki, which needs a highlight cache
  rework.

Two entries left this list by shipping rather than by being dropped: *line-anchored review
comments* became **C2** (SQLite, anchored on change ids) and *forge PR review* became
**C4** (`gh`, GitHub only). The forge entry also carried a stale premise — it said the repo
had moved to tangled.sh and wanted a rethink against tangled's API; it is on GitHub, and C4
was built against `gh`.

## Risks

- **jj CLI output stability** — mitigate with `-T` templates (stable contract), integration
  tests against a pinned jj version matrix (0.43+), and a single parsing module.
- **Working-copy staleness** — we diff the fs directly for `@`, so no snapshot needed for
  viewing; mutations let jj snapshot as usual. Must handle "user ran jj mid-view" via op watcher.
- **Conflicted trees via gix** — detect `.jjconflict-*`, always fall back to `jj file show`.
- **jj-lib churn** — it's Phase 2 and optional; the `crates/vcs` trait keeps it swappable.
- **Diff-view performance** — the real work (codiff's ReviewCodeView is 4k LOC alone). Budget
  M1 generously; virtualize from day one; keep highlighting off the main thread.
- **Shadow-DOM selection** — cross-shadow-root text selection/copy is still uneven
  (`getComposedRanges`); the light-DOM rule for the code pane exists to avoid it entirely.
  Verify select/copy/find UX in M1 before building comments on top.
- **Lit ecosystem thinness** — fewer off-the-shelf widgets than React (e.g. markdown comment
  editor will be hand-rolled). Acceptable for this app's surface; noted so it isn't a surprise.

## Effort

Solo: **~4 weeks to daily-driver (M2), ~8 weeks to M4.** The jj plumbing is the cheap 15%; the
diff UI is the long pole. Everything here is MIT-compatible; borrow UX (not code) from codiff.
