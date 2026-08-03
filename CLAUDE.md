# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

**jjdiff** — a desktop diff viewer for reviewing and landing changes in [Jujutsu](https://jj-vcs.dev) colocated repos. Tauri 2 + Rust workspace + Lit. jj-native: no staging axis, change-id identity, stack review.

Companion docs: [PLAN.md](PLAN.md) (milestones, product theses, what's shipped), [DESIGN.md](DESIGN.md) (**binding** visual spec for anything under `ui/`), [AGENTS.md](AGENTS.md) (a pointer back here).

## Commands

```bash
pnpm install
pnpm tauri dev                          # app against the cwd repo
pnpm tauri dev -- -- -R /path/to/repo   # against another repo
pnpm dev                                # UI only in a browser, backed by ui/src/mock.ts
pnpm build                              # tsc --noEmit + vite build → ui/dist

cargo test --workspace
cargo test -p jjdiff-vcs log_and_stack_roundtrip   # single test
cargo clippy --workspace --all-targets -- -D warnings
```

`pnpm build` must run before any `cargo` command in a clean checkout: `src-tauri/build.rs` fails when `ui/dist` is missing, and it's gitignored. CI enforces this ordering.

There is no JS test runner; `tsc --noEmit` is the only frontend check.

## Architecture

Four Rust crates plus a Lit frontend. Data flows one way: `jj` CLI → `jjdiff-vcs` → `jjdiff-diff` → Tauri command → `ui/src/ipc.ts` → Lit components.

- **`crates/vcs`** (`jjdiff-vcs`) — the only place that shells out to `jj`. `Repo` is cheap to clone (three paths) so commands clone it out of app state and run blocking work off the main thread. `runner.rs` splits reads from mutations; `change.rs` parses JSONL from `-T json(...)` templates.
- **`crates/diff`** (`jjdiff-diff`) — two producers converge on `Vec<FilePatch>`: `parse_git_patch` (from `jj diff --git`) and `worktree::diff_worktree` (live fs vs base tree via gix, so viewing the working copy never snapshots). Both call `assign_hunk_ids` + `spans::add_word_spans`.
- **`crates/watch`** (`jjdiff-watch`) — `notify` watchers on `.jj/repo/op_heads/heads` and the working copy, both emitting `repo-changed`. Non-fatal: without them the app works but won't live-refresh.
- **`src-tauri`** (`jjdiff-app`) — `lib.rs` holds every `#[tauri::command]`; `cli.rs` the headless CLI; `walkthrough.rs`, `describe.rs`, `comments.rs`, `viewed.rs`, `config.rs` the review state.
- **`ui/src`** — `app.ts` is the shell and owns nearly all state; `patch-view.ts` renders the diff; `rows.ts` flattens files/hunks/lines into one `Row[]`; `ipc.ts` is the typed mirror of `lib.rs`; `overlay.ts` the base class every modal extends; `themes.ts` the palette registry. `pr-templates.ts` is the proposal identity row as **plain functions**, never a custom element — it renders directly above the diff, where a shadow root would sever `theme.css`.

### Command shape (`src-tauri/src/lib.rs`)

Commands that touch `jj` or the filesystem are `async` and wrap their work in `blocking()` — a sync Tauri command runs on the main thread and a slow `jj` call freezes the window. Every mutation goes through `run_mutation`, which returns jj's narration plus the operation id and emits `repo-changed`.

Adding a command means three places: the `#[tauri::command]` fn, the `generate_handler![…]` list, and the wrapper + types in `ui/src/ipc.ts`.

**Undo is `jj op revert <that operation>`, never `jj undo`.** That id is what puts an Undo on screen (`App.renderOutcome`). `jj undo` unwinds whichever operation is latest, and an outcome card can outlive being the tip — another window mutates, the watcher refreshes, and the narration is still on screen. Reverting by id is also what makes the same button correct in the operation log, where "Revert this operation" steps one operation back without unwinding the ones after it (the difference from "Restore here" beside it). Neither routes through `confirmImmutableRewrite`: it takes a `Change`, an operation is not one, and `op_revert`/`op_restore` are `mutate` rather than `mutate_rewriting`.

### Change id is the key; commit id is the revset

**Review state is keyed by change id.** Product thesis, not implementation detail. Viewed files, "last reviewed" commits, walkthroughs (`viewed.rs`) and inline comments (`comments.rs`, SQLite) key on `(repo root, change id)`, so they survive `describe`/`squash`/rebase. `CommentStore::refresh_anchors` re-anchors by line content and marks unmatched comments **outdated** rather than dropping them; walkthroughs compare `diff_fingerprint` and are flagged stale.

**A revset is a commit id, never a change id** — the opposite answer to a question that looks the same. A change id survives rewrites, which makes it the right *key* and the wrong *argument to jj*: a **divergent** change (one change id, several visible commits) does not resolve, and jj refuses every command taking one — diff, file show, evolog, resolve, edit, describe, rebase, duplicate, abandon, split, squash, bookmark set. `revisionOf(change)` in `app.ts` is the one place that answer lives; `App.revsetFor` is it plus the working copy's `null` (meaning "diff the live filesystem"). `interdiff_since_reviewed` is the one command wanting both and names them apart (`change_id` the key, `to_commit` the revision). Pinned by `a_divergent_change_resolves_by_commit_id_and_not_by_change_id`.

### Divergence is a state the app names, not one it hides

Being *operable* under divergence is the rule above, and it was for a long time the whole of it: nothing on screen ever said "divergent", so two rows carried the same id, the same description and no marker, and the state was one you had to leave the app to discover. `Change.divergent` comes from `LOG_TEMPLATE` (jj's own keyword, true on **every** side because it describes the id rather than a winner), and the graph row and the detail card both badge it — the card adding the commit id, since that is the string `jj abandon` will take.

**Selection is a `(changeId, commitId)` pair** (`Selection` in `app.ts`), resolved by `App.resolveSelection`: exact match across `stack` and `graph` first, then change id alone. Both halves are load-bearing and neither alone will do. Without the commit id the two sides of a divergent change are one thing the UI cannot point at — both rows lit up and the pane showed whichever the log listed first, so the lower row was clickable and unselectable. Without the change id a selection would not survive the rewrite it just triggered, which is the ordinary case. The exact pass must finish over *both* lists before either falls back, or a loose hit in `stack` beats the exact one waiting in `graph` — precisely the divergent case, whose second side is usually off-stack. `jj-log-graph` takes a **commit id** for the same reason, and the app hands it the resolved change's, not the selection's own.

**Naming the state is not resolving it**, so `App.renderDivergenceBanner` is where the versions exist at once: each side selectable (the diff follows), each comparable against the one on screen, each keepable. Three things make it necessary rather than decorative. The sides are usually *not* both on the graph — the default revset is `ancestors(@ | bookmarks())` and a sibling is typically neither — so they come from `Repo::commits_of_change`, a `jj log -r 'change_id(<id>)'` (jj 0.31, under our 0.33 floor) asked only when the flag is set. **The Versions drawer does not answer this**: `jj evolog` walks a commit's own predecessors, and two divergent sides list their shared ancestor and never each other. And their own diffs look nearly identical, both usually sitting on the same parent, so **compare** is an interdiff between the two commits — the only view that says what actually differs. `keepOneSide` abandons the others in **one** `jj abandon` with a union revset: one operation, one Undo, where a call per side could leave a half-resolved change behind.

**Review state stays keyed by change id, so the two sides share it** — decided, not inherited (`ReviewStore::key`, pinned by `both_commits_of_a_divergent_change_read_one_set_of_notes`). Keying the commit instead would make the sides independent and would throw away every note on the next `describe` of any change anywhere: the failure the key exists to prevent, imposed everywhere to serve a rare state. It also matches how divergence ends — you abandon a side, and shared notes stay with the commit that remains. Crossing over costs only what `refresh_anchors` already bounds: a comment re-anchors by line content or is marked **outdated**, which is the honest answer when the other side does not have that line.

### The binary is the CLI

`main.rs` parses argv with `cli::Args` before `tauri::Builder` exists. Headless commands (`--help`, `--version`, `--diff`, `--print-hunks`, `--walkthrough-guide`, `--install-terminal-helper`) write to stdout and `exit(0)` without creating a window. The same parser handles `second-instance` argv, so opening a second repo reuses the running process instead of forking a rival review store.

`--print-hunks` exists for agents: it dumps `<path>#<index>` ids a walkthrough JSON can reference (see `skills/jjdiff/SKILL.md`). It and `--diff` go through the same `compute_diff` the app uses, and read `[ui] ignore-whitespace` from the user's config rather than assuming it off — hunk ids are positional and `--ignore-all-space` renumbers every later hunk in a file, so a mismatched dump names different hunks than `import_walkthrough` resolves, and an id that misses is dropped silently.

## Invariants

**Reads never snapshot.** Every read in `crates/vcs` uses `--ignore-working-copy --color=never --no-pager`, so it can't contend for the working-copy lock or write to the op log. Mutations deliberately omit those flags. Don't add a read path that skips this.

**Structured output comes from `json()` templates**, never from parsing human-formatted `jj` output. Requires jj ≥ 0.33 (`MIN_JJ_VERSION`); `check_version` fails fast. `JJDIFF_JJ_PATH` overrides the binary. `Repo::op_diff` is the one read returning prose because `jj op diff` has no `json()` form — don't add a parser for it.

**Repos must be colocated** (`.git` inside the workspace) — otherwise `VcsError::NotColocated`.

**The worktree walk must agree with `jj status` exactly.** `worktree::collect_worktree` has twice listed files jj does not, surfacing as changes to code nobody touched. Two rules came out of it. An **ignore rule applies to untracked files only**, so `restore_tracked` puts back every base-tree path the walk never visited — the walker prunes an ignored *directory* without descending, so a tracked file under it is never seen and comes out as a deletion. And a **directory with its own `.git` is another repository**: jj does not snapshot it, so the walk prunes it. Check anything new here against `jj status` on a real repo, not a fixture.

**Span offsets in `Line.spans` are UTF-16 code units**, to match JS string indexing.

**Light DOM above the diff pane.** `jj-app` and `jj-patch-view` override `createRenderRoot()` to return `this`. A shadow root anywhere above a diff row severs `theme.css` and breaks cross-row text selection. Shadow DOM is fine for leaf widgets.

**A backtick inside a `css` *or* `html` tagged template ends it.** Four times now, always in a comment explaining a rule, and the error points at some unrelated method rather than the comment. An `<!-- … -->` is still inside the literal. Write marker syntax in both without backticks.

**No margins on virtualized rows.** All rows are one flat `virtualize()` list measured by `offsetHeight`; gaps come from padding or transparent borders inside the row.

**`IN_TAURI` gates all IPC.** `ipc.ts` falls back to `mock.ts` via dynamic import, so `pnpm dev` works in a plain browser. New IPC wrappers need a mock arm or the browser path breaks.

## Testing notes

`crates/vcs` and `crates/diff` tests shell out to real `jj`. They skip themselves when `jj` isn't installed and serialize through a `JJ_LOCK` mutex — concurrent `jj git init` is flaky. New jj-backed tests must take that guard and set `signing.behavior=drop` plus `JJ_USER`/`JJ_EMAIL`.

CI is split in two (`.github/workflows/`): `crates.yml` for the pure crates, `app.yml` for the Tauri app + UI bundle.

## A `Repo` is a workspace, and three paths are not one path

`Repo::discover` resolves **three** directories that coincide only in a single-workspace repo:

- **`root`** — the workspace (`jj root`): where the files are, what `@` means.
- **`repo_dir`** — the shared `.jj/repo`. A *directory* in the workspace `jj git init` made, and a **file** holding a relative path to it in every workspace `jj workspace add` made since. `op_heads_dir()` hangs off this; built from `root` it would path through a file and the op watcher would silently never fire.
- **`git_dir`** — from `<repo_dir>/store/git_target`. gix reads objects here while `diff_worktree` walks files in `root`, which is why that function takes both.

**Colocation is a property of the store, not of the directory.** `root.join(".git").exists()` was the same question until a second workspace existed — `jj workspace add` gives the new tree a `.jj` and nothing else. Now: resolve `git_target` and reject it if it points *inside* `.jj/repo` (jj's bare store, `git.colocate=false`).

**`review_key()` is the repo, `root()` is the workspace.** Viewed flags, comments, "last reviewed" and walkthroughs key on the former, so review state follows a change between workspaces. In a single-workspace repo it is byte-identical to `root`. `emit_repo_changed` matches on it too: the op log is repo-wide, so a commit in one workspace makes every other window stale.

`fetch_forge_ref` passes `--git-dir` *and* sets the working directory to that git dir's parent — `--git-dir` says where to fetch into and nothing about resolving a remote given as a relative path, which git does against the cwd.

## Workspaces are places, and jjdiff owns only the ones it made

`jj workspace add`'s `-r` does **not** check a revision out: it makes those revisions the *parents* of a new working-copy commit, like `jj new`. So "work on this change in a new workspace" is `add` then `edit`, while "new change on top of it" is the single `add -r`. `checkoutElsewhere` keeps them apart; conflating them hands the reviewer an empty change instead of the one they picked.

Generated workspaces live at `<[workspace] root>/<repo dirname>/<name>`, defaulting to `~/.jjdiff/workspaces`. That prefix is the whole of `workspaces::is_generated`, which decides whether jjdiff may **delete** a directory. `jj workspace forget` never touches the disk, so removing files is jjdiff's own act and it performs it only inside the directory it owns. The forget is undoable and the deletion is not, which is why they are two questions and the deletion happens second.

`WorkspaceView.generated` is computed in the backend rather than re-derived in the UI, since the rule depends on a config value. `forget_workspace` re-checks it regardless — the flag shapes the affordance, the backend is the guarantee.

## Windows own their repo

`AppState` holds no `Repo` — each window does (`WindowState`, keyed by window label), because one window shows exactly one repository. Every repo-touching command takes `window: tauri::Window` and resolves through `repo_handle(&state, &window)`; skipping it would silently read whichever repo the map happened to hold. `repo-changed` is emitted per repo root, so two windows on one repo both refresh and windows on other repos don't.

A command needing only the *key* — viewed flags, reviewed commits, walkthroughs, comments — goes through `repo_key(&state, &window)`, which reads the root off the bound `Repo` and runs no subprocess. It is one function because the string is a *stored* contract with `ReviewStore` and `CommentStore`: a site that spelled it differently (canonicalising the path, or using `launch.repo_path`, which may be a subdirectory) would file review state under a name nothing else reads, losing notes and viewed flags with no error anywhere. Changing the format means a migration; `reads_state_stored_under_the_documented_key` parses a hand-written `review.json` so a change to `ReviewStore::key` fails loudly.

The review and comment stores stay app-global — both are already keyed by repo root, so two windows on one repo must share them.

**A window that changes repo resets through `App.resetRepoState`.** Both routes in (`switchRepo`, `handleSecondInstance`) re-bind the backend immediately, so every field derived from the old repo is wrong the moment they return. Clearing is up front rather than left to the loaders, because a slow or failed loader leaves the previous value on screen. The quiet failures were worse: comments are keyed by path, so one repo's notes landed on same-named files in another, and an inherited proposal index could raise the old repo's PR banner over the new repo's code. Forge discovery reruns too (`loadForge`).

`RepoScoped` is a union of every per-repo field and `freshRepoScope()` builds one value for each; the factory `satisfies Record<RepoScoped, unknown>`, so a name with no value and a value with no name are both compile errors. It must be a function, not a shared constant: half the defaults are a `new Map()` or `new Set()`, and a literal would hand every reset the same instance. It cannot catch a field added to *neither*, **so adding per-repo state means adding it to `RepoScoped`**. Three stay out deliberately: `busy` (clearing it mid-flight re-enables the toolbar under a running mutation), `detailCollapsed` (sticky on purpose) and the flags a `finally` owns.

Window labels are `main` plus `repo-N`. `capabilities/default.json` must cover that pattern; a label outside it gets no IPC at all, which presents as a window that loads and then does nothing.

## The menu mirrors the palette

`ui/src/app.ts` owns the one command list and pushes it through `set_menu`; `menu.rs` turns each entry into a menu item carrying its command id, and a click emits `menu-command`. Don't add commands to `menu.rs` — add them to the palette and they appear in both. **The Edit submenu is required**, not decoration: a custom menu without it strips Cmd+C/Cmd+V from the WebView on macOS. Mirrored items carry no accelerators, because the frontend dispatches shortcuts itself and a menu accelerator would shadow or double-fire them.

## Every action goes through one helper

`App.act(opts, action)` owns three things: the re-entry guard, the `busy` label, and what a failure does to what is on screen. `command()` (a jj mutation — narrate, refresh, reload the op log) and `run()` (everything else) are thin wrappers, and everything goes through one of them; the four methods that used to hand-roll it each dropped the guard, so one finishing under an in-flight mutation cleared `busy` while that mutation was still running.

`busy` is a **label**, not a flag, because it is read for identity (the review composer disables itself on `busy === 'submit-review'`). An action refused because something else is running says so in `actionInfo` rather than returning bare — a silent no-op is the failure mode nobody can file a bug about. Failures land in `report(error)`, one policy. Success payloads stay the caller's: `openProposal` rolls its banner back in a *nested* catch, because `act` cannot tell a failure from a refusal.

## Themes are derived, not written

`ui/src/themes.ts` seeds each named palette from about a dozen colours and computes the rest of the `--jj-*` token set; only light/dark are hand-written, in `theme.css`. They apply as **inline custom properties on `:root`**, which is how a named palette beats both the `:root` block and the `prefers-color-scheme` block — `system` clears them and control returns to the media query.

Two data attributes: `data-jj-theme` is the *mode* (all theme.css needs), `data-jj-palette` the *identity* (what `highlight.ts` needs to pick a shiki theme). A seed whose `shiki` has no entry in `THEME_LOADERS` silently falls back to `github-dark`, so adding a theme means touching both.

## Config

`~/.config/jjdiff/config.toml` (`config.rs`). TOML keys are kebab-case via serde aliases while the JSON sent to the UI is camelCase. An unreadable config logs and falls back to defaults — it must never block startup. Walkthrough and description generation shell out to an agent CLI (`claude` by default; `codex`, `opencode`, `pi` selectable) with the prompt on stdin.

Writes go through `config::set_setting` → `write_item`, which uses `toml_edit` to touch exactly one key: the file is the user's, and round-tripping it through `Config` would delete their comments, key order and any setting a newer jjdiff added.

**One write path, allow-listed and typed.** `set_setting` refuses a table/key not in `WRITABLE`, and a value whose JSON type does not match the declared one. Both matter. The WebView renders forge markdown from anyone who can open a pull request, so a passthrough taking an arbitrary table and key would let that write anywhere in the user's config. And TOML is typed while serde is not forgiving: `ignore-whitespace = "true"` deserializes as a bool nowhere and takes the whole `[ui]` table down to defaults on the next load.

**Settings are `⌘,`, and the palette writes too.** `App.applySetting` is the single path that applies a setting live *and* persists it; the palette's Diff Layout / Word Wrap / Whitespace toggles call it as well, having previously flipped a field and nothing else. `App.config` is the local copy the page renders from, updated before the write so the UI does not wait on disk; a failed write reports and leaves the setting live. `CAMEL` in `app.ts` crosses the kebab/camel boundary when updating that copy.

`[editor] command` drives "Open in Editor" (`o`): a template with `{file}`, `{line}` and `{repo}`. It is split on whitespace *before* substitution and executed with no shell, so a path containing spaces stays one argument and nothing in a filename can inject another — keep that order if you touch `editor.rs`.

## Forge review

`forge.rs` drives `gh` as a CLI, never REST — jjdiff handles no tokens. **GitHub only:** a GitLab path existed and was deleted rather than kept, having been written against `glab`'s documented JSON and never run against a live instance. `Kind` stays an enum so restoring it is a variant, not a rewrite. The forge is inferred from the remote URL; a host it can't place is an error rather than a guess, and the UI hides forge affordances entirely rather than offering ones that fail. Every parser is tested against fixtures captured verbatim from real `gh` output.

**Never diff a proposal with `base..head`.** Once merged, the head is an ancestor of the base branch and that revset is silently *empty* — a review showing nothing, with no error. Use the forge's own merge base (`baseRefOid`), correct for open and merged proposals alike. `open_pull_request` also fetches the base branch so that OID resolves locally.

**And never read a file at that revset either.** `getFileContent`/`getFileBytes` run `jj file show -r`, which refuses a revset resolving to more than one revision, and `image-view` derives the old side as `${revset}-` — for `base..head`, the nonsense `base..head-`. Context expansion and the markdown preview go through `App.contentRevset`, which resolves to a **single** revision: the proposal's head while the pane shows a whole proposal, otherwise the selected change's commit. Which spelling of the head depends on what the repo holds — a bookmark that matched the index is local by construction and wins when the graph carries it; `headOid` is the forge's exact answer and what `openProposal` fetched onto `jjdiff-pr-N`, so it wins otherwise. `reviewTarget` answers a different question: the *key* review state hangs off, whose `revset` is a scope to diff.

**A proposal is context on a change, not a mode.** Every `Change` carries its bookmarks and `gh pr list` returns each proposal's head branch, so selecting a change with a matching bookmark surfaces its banner automatically. The index loads once per repo (a network call), not per selection.

**The index answers "what is open here", never "does this branch have a proposal".** It is one page of *open* proposals, so on a busy repo a branch's own proposal is routinely outside it and a merged one is never in it. When it doesn't cover a bookmark, `App.lookupProposalByBookmark` asks the exact question (`Client::find_by_head`). It stays a **fallback**: the index is free, this costs a subprocess. `branchProposals` caches **promises**, so overlapping refreshes share one call instead of racing, and it caches misses as well as hits or an unproposed branch is re-asked on every selection; a *failed* call is evicted, since it is not an answer. The cache drops whenever the index reloads. `jjdiff pr N` covers a proposal whose branch is not local by fetching it. `prRevset` is set only while the diff shows the *whole* proposal — a toggle, not a screen.

**The proposal is a view, not a panel.** The banner is an *indicator* — state, checks, drift, one line — and clicking it switches `viewMode` to `'pr'`, where the description and conversation live. Two consequences: `main` is a flex column, so a view that does not claim `flex: 1; min-height: 0` gets shrunk to a fraction of the pane, and its siblings are hidden by `main.showing-pr > *:not(.pr-view)` rather than unmounted individually. The body and conversation load **when the view is opened** — two extra `gh` calls most selections never need — and cache until a refresh clears `prDetailsFor`.

**Opening a proposal touches nothing jjdiff watches.** `gh pr create` in a terminal writes no jj operation and no file, so neither watcher fires. The index is refreshed on **window focus** (throttled by `PROPOSAL_REFRESH_MS`) and after every **push**. Both `loadProposalIndex` and `syncMatchedProposal` keep their previous value on failure, or a visible banner would vanish on one flaky call.

**Creating one is two steps, in order: push, then `gh pr create`.** `gh` resolves the head branch against what the *forge* can see, so a bookmark that exists only locally fails there talking about a branch that plainly exists. `App.submitNewProposal` sets the bookmark if needed, pushes, then creates; the outcome carries no operation id, because reverting the push would leave the proposal open pointing at a branch the remote no longer has. The compose dialog seeds its fields **once, on mount** — bound from the parent they would be reassigned on every watcher tick and window focus, reverting a description mid-sentence.

`Repo::fetch_forge_ref` is the one place jjdiff shells out to **git** instead of jj: proposal heads live outside `refs/heads/*` and `jj git fetch` takes bookmark globs, not refspecs. Safe only because `discover` guarantees colocation. The head lands on a `jjdiff-pr-N` bookmark, after which a proposal is an ordinary revset.

**A proposal's conversation lives in three places** and only reads as one thread merged and sorted by time: issue comments and reviews from `gh pr view --json comments,reviews`, line-anchored comments only from `gh api …/pulls/N/comments`. `Client::activity` merges them. Two details that look like bugs otherwise: submitting inline comments creates a **review row with no verdict and no body** purely to hang them on (filtered out, or it renders blank), and an inline comment's `line` goes **null** once its anchor leaves the diff, so `original_line` keeps it attached to a real place.

**Forge markdown is untrusted and goes through `ui/src/markdown.ts`.** A proposal body is arbitrary text from anyone who can open a PR, rendered in a WebView that can invoke every Tauri command, and `marked` does not sanitise. The CSP is a backstop and is **absent under `pnpm dev`** — relying on it would make the browser build the exploitable one. The scrubber is an allow-list (unknown elements unwrap, keeping their text); `<script>`/`<style>` are dropped outright, or their source renders as prose. Links are plain `<a href>` handled by a delegated listener.

## The WebView has no dialogs

`alert()`, `confirm()` and `prompt()` do not work — wry's `WKUIDelegate` implements only the file-open panel, `windowWillClose` and media permissions. On macOS `prompt()` returns `null`, `confirm()` returns `false`, and `alert()` does nothing, so every action gated on one is a **silent no-op** with no error anywhere. This shipped: abandon, discard, delete-bookmark, rebase and create-bookmark were all dead in the app while working fine in `pnpm dev`. Use `askText` / `askConfirm` from `ui/src/prompt.ts`, which also work in the browser mock.

Same class of gap: **`target="_blank"` does nothing** — there is no tab to open. Outbound links go through the `open_url` command, which hands the URL to the OS and refuses any scheme that isn't http/https.

And **drag and drop needs Tauri's own handler turned off** — `dragDropEnabled: false` in `tauri.conf.json`, `.disable_drag_drop_handler()` on every window `spawn_window` builds. That handler exists so a file dropped from Finder can raise an event, and it takes over the WebView as an OS drag destination; wry forwards the AppKit drag to WebKit only when the handler declines, and Tauri's never declines. An in-page drag is an AppKit drag as well, so `dragstart` fired (the source side is all WebKit) and `dragover` and `drop` never did — rebase-by-drag and moving a bookmark onto another change both looked implemented and were dead in the app while working in `pnpm dev`. Costless here: jjdiff listens for no file drop, taking its repo from argv and from a second instance.

### Overlays and event retargeting

An event from inside a shadow root is **retargeted to the host** on the way out, which breaks two things and is why `ui/src/overlay.ts` exists.

- **A backdrop test is `event.composedPath()[0] === this`, never `event.target === this`** — the scrim *is* the host, so a click on the panel is otherwise indistinguishable from a click outside it.
- **An overlay with a text field binds keys on the panel** (`OverlayElement.onPanelKey`), because `App.onGlobalKey` decides an event is typing by reading `event.target.tagName`, which by then is the host and not the input two shadow roots down — so `j`, `k`, `c`, `v` typed into a filter box would scroll the diff behind the dialog. Escape is let through: the window listener owns it.

Both, plus the window Escape listener, are written once and every overlay extends `OverlayElement`. It is a base class and a set of `CSSResult`s — **never a custom element**, because these mount above the diff pane. It deliberately does not own the way out: `dismiss()` is abstract, since the paths differ (most dispatch `close`, `jj-theme-picker` must re-emit `preview-theme` first or the last hovered palette stays applied, `jj-prompt` resolves the promise `askText` awaits). `escapeOnWindow` is off for the three whose Escape belongs elsewhere.

On the app side the question is asked once: **`App.overlayOpen` is the single guard `onGlobalKey` returns on**, replacing two hand-maintained flag lists that had drifted. An open overlay swallows the walkthrough arrows and the Escape chain as well as the single-letter review keys. The shortcuts sheet is deliberately outside it (`?` toggles it and Escape closes it from that same handler), as is `jj-prompt`, which mounts itself and stops every key on its panel. **Adding a new overlay means adding its flag to that getter**, and to nothing else.

## Ahead/behind is reported inverted by jj

`Repo::bookmark_statuses` reads `tracking_ahead_count` / `tracking_behind_count`, and **both mean the opposite of what the fields in `BookmarkStatus` mean**. The keywords live on the *remote* ref and describe the remote's position — `jj bookmark list` prints `@origin (behind by 2 commits)` when your local branch is two commits **ahead**. jjdiff states everything from the local side (`ahead` = a push would send these), so the mapping is crossed exactly once, in that function. Read straight, the badges are backwards in a way that looks entirely plausible; `bookmark_statuses_report_ahead_and_behind_from_the_local_side` drives both directions against a real remote. The synthetic `git` remote of a colocated repo is filtered out — in sync by construction, so reporting it is noise.

**`Repo::unpushed` is the other half.** A change with *no* bookmark tracks nothing, so it has no ahead count and appears in no `BookmarkStatus` however long it goes unpushed. `remote_bookmarks()..` covers it, with two guards. A repo with **no git remote** gets an empty answer rather than its whole history — `x..` is `x..visible_heads()`, so an empty left side excludes nothing. And the **empty undescribed working copy** is excluded, or the indicator is on permanently.

The one place ahead/behind matters beyond a badge is the PR banner: `headDrift` warns when the proposal's head branch has unpushed commits, because CI, reviewers and merge state all describe the head *the forge has*. A green "checks passed" beside unpushed work reads as approval of code the forge never saw.

## Rewriting immutable commits is per-call, never a mode

`Repo::allowing_immutable(true)` returns a handle whose **rewriting** verbs carry `--ignore-immutable`; `mutate_rewriting` applies it and `mutate` does not. The split is not cosmetic: `backout` and `duplicate` *reject* the flag (they add commits rather than rewrite them), so funnelling every mutation through one helper turns an unrelated command into a clap parse error.

The opt-in is a per-call argument all the way from `ui/src/ipc.ts` to the CLI — nothing stores it, because a persisted toggle would hand back jj's accident guarantee for a whole session instead of one confirmed command. On the frontend every such action routes through `App.confirmImmutableRewrite`, which returns `true` immediately for mutable changes and otherwise names the bookmark, the force-push and the descendants that get rebased. Adding a new rewriting command means threading `allowImmutable` through and calling that helper — one that skips it silently rewrites published history.

## Hunk-level split and squash: jjdiff is jj's diff editor

`jj split -i` has no non-interactive form and no flag that takes hunks. What it has is a protocol: jj writes the two sides into a pair of directories, runs the configured diff editor, and takes whatever the **right** one holds when it exits. So jjdiff plays the editor. `Repo::split_with_diff_editor` registers this binary as `merge-tools.jjdiff-split` for one invocation (`--config`, never written to anyone's config file) and jj re-enters it as `jjdiff --apply-split-plan <plan> $left $right`. `-m` is not optional there: without it jj opens `$EDITOR` for the description, and a GUI-spawned editor with no terminal hangs.

The plan is built by the frontend from the diff **on screen** (`App.buildHunkPlan`), so the hunks that move are the ones that were ticked. That is only safe because `apply_selected_hunks` refuses to write unless two things hold: every hunk's context and removed lines appear verbatim where it claims, and applying *all* of them reproduces the right side exactly. The second is load-bearing — it is a property of any correct diff, so it costs nothing when things are well and catches a stale plan, a file edited since it was read, or a diff of some other pair of trees. It also means our hunk boundaries need not match jj's. A failure exits non-zero and jj aborts rather than committing a half-edited directory.

Both guards hold verbatim, but over **lines, not bytes**. A plan line never carries a `\r` (it comes from `jj diff --git` parsed with `str::lines`) while jj hands the diff editor the file's own bytes, so every hunk of a CRLF file failed the first guard on a terminator nobody touched. `apply_selected_hunks` strips the `\r` from both sides when **every** line of both carries one, and re-appends it when writing. A file with *mixed* endings is compared exactly as it sits and the split is refused, because agreeing with the plan there would rewrite the terminators of lines the reviewer never selected.

Three cases the plan encodes rather than derives: `select: "all"` is left exactly as jj wrote it, `"none"` is restored from the left side (one rule — copy what the old side has, remove what it does not — which undoes an edit, an addition, a deletion and, with a rename's two paths, a rename), and only `"hunks"` runs the arithmetic. `supportsHunkSelection` in `rows.ts` decides which files can be divided at all: not binary, not renamed, more than one hunk.

**`jj squash -i` is the same protocol over the same trees**, which is why one plan serves both verbs and `--apply-split-plan` is not renamed: a squash's editor is handed the *source's own* diff, and jj squashes whatever the right directory holds into the destination. Three things differ, all load-bearing: `SplitPlan::moves` replaces `divides` (a squash may take every hunk; a split must leave something behind), `--use-destination-message` is not optional (jj's default combines both descriptions through `$EDITOR`, which hangs with no terminal), and both ends are checked for immutability because a squash rewrites the destination too.

## The walkthrough overview is a document, not a summary

A walkthrough has two halves. The **steps** are a reading order over the diff — title, narrative, hunk ids. The **overview** (`Walkthrough::overview`, markdown) is synthetic: impacted systems as a mermaid `flowchart`, one section per changed system boundary with its routing and a `diff` fence of the contracts that moved, then tables of new mutable state and new effects. It is what a reviewer needs *before* the first hunk means anything. `summary` survives as one plain paragraph for places a document does not fit, and as the fallback for walkthroughs stored before overviews existed — `overview` is `Option<String>`, and absent is not an error.

The authoring contract lives in **three mirrored copies** — `walkthrough::GUIDE` (what jjdiff sends an agent), `cli::WALKTHROUGH_GUIDE` and `skills/jjdiff/SKILL.md` — and they must say the same thing: a generated walkthrough and a handed-in one are the same artefact, and a rule only one copy states is a rule half the callers break. Under `outline` (the diff was too big to send) the guide additionally forbids the contract fences, which cannot be written from paths and `@@` headers.

The canonical nesting is `#` title → `##` section → `###` boundary, and that is what the copies drift on. Checked once, in `cli::tests::the_three_authoring_guides_state_one_overview_contract`, newline-anchored because `### X` contains `## X` — one test per copy is what let two passing tests pin `##` and `###` for the same section at the same time.

Because the overview owns the pane (`main.showing-overview`, DESIGN.md §6) there is no diff on screen while it is read, so the **Files:** line is the way back into the code. `App.markFileRefs` decides at render time which inline-code paths name a file in *this* diff and marks only those clickable, so a path jjdiff cannot place stays plain code instead of becoming a dead link.

A figure opens in `jj-diagram-view` carrying the **rendered SVG** rather than the mermaid source, because asking mermaid for the same picture twice is a second chance for it to fail. It is modal over a screen answering the same keys, so `App.onGlobalKey` returns early while it is open — without that, Escape closed the diagram *and* ended the review underneath it.

Rendering goes through `ui/src/markdown.ts`, the same scrubber as forge markdown, for the same reason: an LLM's output is not text jjdiff wrote. `renderMarkdownWithDiagrams` splits on **tokens**, not a regex — the guide's own examples nest a fence inside a fence, and a regex cuts the document in half there. Mermaid is a dynamic import (larger than the rest of the app put together) drawn with `securityLevel: 'strict'` and `htmlLabels: false`, its palette read off the live `--jj-*` tokens so a theme change redraws it; a diagram that fails to parse falls back to its source **and says why**, since a silent fallback is indistinguishable from an agent writing a fence of plain text. Colours reach mermaid through `cssColor`, which resolves `var()`/`color-mix()` on a probe element then *rasterises one pixel* — `getPropertyValue` returns unsubstituted token text and an engine may serialise a resolved colour as `color(srgb …)`; khroma rejects both, so only 8-bit RGBA is safe to hand it.

## A generated commit message is a draft

`describe.rs` shares `walkthrough.rs`'s backend plumbing — the same four agent CLIs, driven headlessly the same way — and nothing else. `[describe] prompt` is its own config key: "always name the ticket" belongs on a message, "flag public API changes" belongs on a review.

The prompt carries the **last five descriptions from the repo it is writing for** (`ancestors(@-,6)`, never `@` — the working copy's own description is the thing being replaced, and offering it as an example would have the agent imitate the placeholder it was asked to improve). Neither jjdiff's prose style nor a monorepo's gitmoji convention is written anywhere a model could look up, and both are obvious from five examples.

`generate_description` **returns the text and describes nothing**. The button fills the box; Describe beside it is what commits. Past `MAX_DIFF_CHARS` the code is dropped and the file list kept — unlike a walkthrough, a message needs to know what moved, not to read every line — and a reply that is prose rather than the requested JSON is taken at face value, because a model that answered with the message has done the task.

## Conflicts are navigable, not just flagged

`Repo::conflicts` keeps both halves of each `jj resolve --list` line — the path *and* jj's description of its shape ("2-sided conflict"), the only place a conflict's arity is stated outside the marker lines. `PatchView.moveToConflict` steps between the `<<<<<<<` lines rather than between files, because one file can hold several; a conflicted file whose contents were not diffed contributes its header as a stand-in so the banner's count and the button's reach agree. Marker lines carry a role class (`start`/`end`/`side`/`base`); jj already names each side in the marker text, so the colour only has to say which *kind* of side it is.

**Resolution is in-app, and it is not a merge editor.** M4's reason for deferring it — a merge editor spawned from a GUI with no TTY hangs more often than it works — was about *spawning* one, and still holds: nothing is spawned. `crates/diff/src/conflict.rs` reads jj's materialized marker text back into structure, `jj-conflict-resolver` offers a side per region, and the answer goes back through `Repo::resolve_with_merge_tool`.

Two things that look like details and are not. **Marker length is per region**: jj lengthens every fence past anything conflict-shaped in the content, so inside a region opened with nine characters a line of seven is content — `parse_region` carries the opening run and ignores anything shorter. And **a resolution that still holds fences is refused**, in `resolve.rs` and again in the command: jj does not re-parse a merge tool's output, so handing it markers writes them into the file and calls the conflict resolved.

## Keyboard shortcuts

Handlers live in `App.onGlobalKey` and `PatchView`; the `?` sheet renders `shortcutReference()` from `keys.ts`. That table is documentation, not dispatch, so a new binding needs both — a shortcut with no entry is one nobody can discover.
