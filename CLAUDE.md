# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

**jjdiff** — a desktop diff viewer for reviewing and landing changes in [Jujutsu](https://jj-vcs.dev) colocated repos. Tauri 2 + Rust workspace + Lit. jj-native: no staging axis, change-id identity, stack review.

Companion docs: [PLAN.md](PLAN.md) (milestones, product theses, what's shipped), [DESIGN.md](DESIGN.md) (**binding** visual spec for anything under `ui/`), [AGENTS.md](AGENTS.md) (shorter duplicate of this file — keep them in sync or drop one).

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

`pnpm build` must run before any `cargo` command in a clean checkout: `src-tauri/build.rs` (tauri-build) fails when `ui/dist` is missing, and it's gitignored. CI enforces this ordering.

There is no JS test runner; `tsc --noEmit` is the only frontend check.

## Architecture

Four Rust crates plus a Lit frontend. Data flows one way: `jj` CLI → `jjdiff-vcs` → `jjdiff-diff` → Tauri command → `ui/src/ipc.ts` → Lit components.

- **`crates/vcs`** (`jjdiff-vcs`) — the only place that shells out to `jj`. `Repo` is cheap to clone (two paths) so commands clone it out of app state and run blocking work off the main thread. `runner.rs` splits reads from mutations; `change.rs` parses JSONL from `-T json(...)` templates.
- **`crates/diff`** (`jjdiff-diff`) — two producers converge on `Vec<FilePatch>`: `parse_git_patch` (from `jj diff --git`) and `worktree::diff_worktree` (live fs vs base tree via gix, so viewing the working copy never snapshots and never writes an operation). Both call `assign_hunk_ids` + `spans::add_word_spans`.
- **`crates/watch`** (`jjdiff-watch`) — `notify`-based watchers on `.jj/repo/op_heads/heads` and the working copy; both emit `repo-changed` to the frontend. Non-fatal: without them the app works, it just won't live-refresh.
- **`src-tauri`** (`jjdiff-app`) — `lib.rs` holds every `#[tauri::command]`; `cli.rs` the headless CLI; `walkthrough.rs`, `comments.rs`, `viewed.rs`, `config.rs` the review state.
- **`ui/src`** — `app.ts` is the shell and owns nearly all state; `patch-view.ts` renders the diff; `rows.ts` flattens files/hunks/lines into one `Row[]`; `ipc.ts` is the typed mirror of `lib.rs`. `orbs.ts` is a pure-presentation leaf (the agent-thinking indicator) with no IPC and no state — DESIGN.md §7 says where it is allowed. `themes.ts` is the named-palette registry.

### Command shape (`src-tauri/src/lib.rs`)

Commands that touch `jj` or the filesystem are `async` and wrap their work in the `blocking()` helper — a sync Tauri command runs on the main thread and a slow `jj` call there freezes the window. Every mutation goes through `run_mutation`, which returns jj's own narration plus the operation id (that's what makes one-click undo possible) and emits `repo-changed`.

Adding a command means touching three places: the `#[tauri::command]` fn, the `generate_handler![…]` list, and the matching wrapper + types in `ui/src/ipc.ts`.

### Review state is keyed by change id, not commit id

This is the product thesis, not an implementation detail. Viewed files, "last reviewed" commits, walkthroughs (`viewed.rs`, JSON in the app data dir) and inline comments (`comments.rs`, SQLite) are all keyed `(repo root, change id)`, so they survive `describe`/`squash`/rebase. When a change evolves, `CommentStore::refresh_anchors` re-anchors comments by line content and marks unmatched ones **outdated** rather than dropping them; walkthroughs compare `diff_fingerprint` and are flagged stale.

### The binary is the CLI

`main.rs` parses argv with `cli::Args` before `tauri::Builder` exists. Headless commands (`--help`, `--version`, `--diff`, `--print-hunks`, `--walkthrough-guide`, `--install-terminal-helper`) write to stdout and `exit(0)` without creating a window. The same parser handles `second-instance` argv, so opening a second repo reuses the running process instead of forking a rival review store.

`--print-hunks` exists for agents: it dumps `<path>#<index>` ids that a walkthrough JSON can reference. See `skills/jjdiff/SKILL.md`.

## Invariants

**Reads never snapshot.** Every read in `crates/vcs` uses `--ignore-working-copy --color=never --no-pager`, so it can't contend for the working-copy lock or write to the op log. Mutations deliberately omit those flags to match the user's own `jj` semantics. Don't add a read path that skips this.

**Structured output comes from `json()` templates**, never from parsing human-formatted `jj` output. Requires jj ≥ 0.33 (`MIN_JJ_VERSION`); `check_version` fails fast so you get a version error rather than a confusing template parse error. `JJDIFF_JJ_PATH` overrides the binary.

`Repo::op_diff` is the one read that returns prose instead of a structure, and it is not an exception to that rule but a consequence of it: `jj op diff` takes no `-T` and has no `json()` form, so jjdiff shows its narration verbatim — the same contract as a mutation's summary — rather than parsing it. Don't add a parser for it; if the UI needs structure there, the answer is a jj feature request.

**Repos must be colocated** (`.git` inside the workspace) — otherwise `VcsError::NotColocated`.

**Span offsets in `Line.spans` are UTF-16 code units**, to match JS string indexing.

**Light DOM above the diff pane.** `jj-app` and `jj-patch-view` override `createRenderRoot()` to return `this`. A shadow root anywhere above a diff row severs `theme.css` from it and breaks cross-row text selection — this shipped as a real bug. Shadow DOM is fine for leaf widgets (file tree, command bar, walkthrough panel).

**No margins on virtualized rows.** All rows across all files are one flat `virtualize()` list; the virtualizer measures `offsetHeight`, so gaps must come from padding or transparent borders inside the row.

**`IN_TAURI` gates all IPC.** `ipc.ts` checks `'__TAURI_INTERNALS__' in window` and falls back to `mock.ts` via dynamic import, so `pnpm dev` works in a plain browser with no jj repo. New IPC wrappers need a mock arm or the browser path breaks.

## Testing notes

`crates/vcs` and `crates/diff` tests shell out to real `jj`. They skip themselves when `jj` isn't installed, and serialize through a `JJ_LOCK` mutex — concurrent `jj git init` is flaky. New jj-backed tests must take that guard and set `signing.behavior=drop` plus `JJ_USER`/`JJ_EMAIL`, as the existing ones do.

CI is split in two (`.github/workflows/`): `crates.yml` for the pure crates (fast, caches a pinned `jj-cli` build), `app.yml` for the Tauri app + UI bundle (pulls the GTK/WebKit stack).

## Windows own their repo

`AppState` holds no `Repo` — each window does (`WindowState`, keyed by Tauri window label), because one window shows exactly one repository. Every repo-touching command takes `window: tauri::Window` and resolves through `repo_handle(&state, &window)`; adding a command that skips this would silently read whichever repo the map happened to hold. `repo-changed` is emitted per repo root (`emit_repo_changed`), so two windows on the same repo both refresh and windows on other repos don't.

The review and comment stores stay app-global on purpose — both are already keyed by repo root, so two windows on one repo must share them.

Window labels are `main` plus `repo-N`. `capabilities/default.json` must cover that pattern; a label outside it gets no IPC at all, which presents as a window that loads and then does nothing.

## The menu mirrors the palette

`ui/src/app.ts` owns the one command list, and which entries exist depends on the selected change. It pushes that list through `set_menu`; `menu.rs` turns each entry into a menu item carrying its command id, and a click emits `menu-command` for the frontend to run. Don't add commands to `menu.rs` — add them to the palette and they appear in both. The app/File/Edit/Window submenus are Tauri predefined items; **the Edit submenu is required**, not decoration, because a custom menu without it strips Cmd+C/Cmd+V from the WebView on macOS. Mirrored items carry no accelerators on purpose: the frontend dispatches shortcuts itself and a menu accelerator would shadow or double-fire them.

## Themes are derived, not written

`ui/src/themes.ts` seeds each named palette (Nord, Catppuccin, Ayu, Rosé Pine, …) from about a dozen colours and computes the rest of the `--jj-*` token set from them; only light/dark are hand-written, in `theme.css`. They are applied as **inline custom properties on `:root`**, which is how a named palette beats both the `:root` block and the `prefers-color-scheme` block in that file — `system` clears them and control returns to the media query.

Two data attributes, answering different questions: `data-jj-theme` is the *mode* (all theme.css needs), `data-jj-palette` is the *identity* (what `highlight.ts` needs to pick a matching shiki theme). Every seed names a shiki theme, loaded on demand by `highlight.worker.ts`; a seed whose `shiki` has no entry in `THEME_LOADERS` silently falls back to `github-dark`, so adding a theme means touching both.

## Config

`~/.config/jjdiff/config.toml` (`config.rs`). TOML keys are kebab-case via serde aliases while the JSON sent to the UI is camelCase. An unreadable config logs and falls back to defaults — it must never block startup. Walkthrough generation shells out to an agent CLI (`claude` by default; `codex`, `opencode`, `pi` selectable) with the prompt on stdin.

Writes go through `config::set_value`, which uses `toml_edit` to touch exactly one key — the file is the user's, and round-tripping it through `Config` would delete their comments, key order and any setting a newer jjdiff added. `set_editor_command` and `set_ui_theme` are both one-line wrappers over it.

`[editor] command` drives "Open in Editor" (`o`, or the file-tree context menu): a template with `{file}`, `{line}` and `{repo}`. It is split on whitespace *before* substitution and executed with no shell, so a path containing spaces stays one argument and nothing in a filename can inject another — keep that order if you touch `editor.rs`.

## Forge review

`forge.rs` drives `gh` as a CLI, never REST — jjdiff handles no tokens. **GitHub only:** a GitLab path existed and was deleted rather than kept, because it was written against `glab`'s documented JSON and never run against a live instance — shipping the appearance of support. `Kind` stays an enum so restoring it is a variant, not a rewrite, and it is recoverable from history. The forge is inferred from the remote URL; a host it can't place is an error rather than a guess, and the UI hides forge affordances entirely rather than offering ones that fail.

**Never diff a proposal with `base..head`.** Once merged, the head is an ancestor of the base branch and that revset is silently *empty* — a review showing nothing, with no error. Use the forge's own merge base (`baseRefOid`, or GitLab's `diff_refs.base_sha`), which is correct for open and merged proposals alike. `open_pull_request` also fetches the base branch so that OID resolves locally.

**A proposal is context on a change, not a mode.** `gh pr list` returns each proposal's head branch and every `Change` carries its bookmarks, so selecting a change with a matching bookmark surfaces its banner automatically — CI, reviewers and merge state show up while you work on your own branch, without asking. The index is loaded once per repo (it is a network call), not per selection. `jjdiff pr N` exists for the other case: a proposal whose branch is not local. It fetches, and the banner then arrives through the same match. `prRevset` is set only while the diff shows the *whole* proposal instead of the selected change — a toggle, not a screen.

**The proposal is a view, not a panel.** The banner in the diff pane is an *indicator* — state, checks, drift, one line — and clicking it switches `viewMode` to `'pr'`, which is where the description and conversation live. They were briefly a disclosure above the diff; prose of unbounded length hanging over the code meant the thing being reviewed started halfway down the window. Two consequences worth knowing: `main` is a flex column, so a view that does not claim `flex: 1; min-height: 0` gets shrunk to a fraction of the pane, and its siblings (detail card, banners, breadcrumb) are hidden by `main.showing-pr > *:not(.pr-view)` rather than unmounted individually. The body and conversation load **when the view is opened**, not with the banner — two extra `gh` calls that most selections never need — and are cached until a refresh clears `prDetailsFor`.

**Opening a proposal touches nothing jjdiff watches.** `gh pr create` in a terminal, or the "create a pull request" link a push prints, writes no jj operation and no file — so neither watcher fires and the banner would stay missing until a manual reload. The index is therefore refreshed on **window focus** (throttled by `PROPOSAL_REFRESH_MS`, since focus fires on every alt-tab and each call is a `gh` subprocess) and after every **push**, which either creates the branch a proposal attaches to or moves an existing one's head. Because `loadProposalIndex` now runs in the background, both it and `syncMatchedProposal` keep their previous value on failure: clearing was harmless when the index loaded once per repo, but as a refresh it would make a visible banner vanish on one flaky call.

`Repo::fetch_forge_ref` is the one place jjdiff shells out to **git** instead of jj: proposal heads live outside `refs/heads/*` and `jj git fetch` takes bookmark globs, not refspecs. It's safe only because `discover` guarantees colocation. The head lands on a `jjdiff-pr-N` bookmark, after which a proposal is an ordinary revset — same diff pane, comments and walkthroughs as any change.

Every parser is tested against fixtures captured verbatim from real `gh` output on this repo's own pull requests.

**A proposal's conversation lives in three places** and only reads as one thread once merged and sorted by time: issue comments and reviews come from `gh pr view --json comments,reviews`, while comments anchored to a line only come from `gh api …/pulls/N/comments`. `Client::activity` merges them. Two details that look like bugs otherwise: submitting inline comments creates a **review row with no verdict and no body** purely to hang them on (filtered out, or it renders as a blank entry), and an inline comment's `line` goes **null** once its anchor leaves the diff, so `original_line` is the fallback that keeps it attached to a real place.

**Forge markdown is untrusted and goes through `ui/src/markdown.ts`.** A proposal body or comment is arbitrary text from anyone who can open a PR, rendered in a WebView that can invoke every Tauri command. `marked` does not sanitise. The CSP (`default-src 'self'`) blocks inline handlers, but it is a backstop and is **absent under `pnpm dev`** — relying on it would make the browser build the exploitable one. The scrubber is an allow-list (unknown elements unwrap, keeping their text); `<script>`/`<style>` are dropped outright rather than unwrapped, or their source would render as prose. Links are plain `<a href>` handled by a delegated listener, because `target="_blank"` does nothing here.

## The WebView has no dialogs

`alert()`, `confirm()` and `prompt()` do not work. wry's `WKUIDelegate` implements only the file-open panel, `windowWillClose` and media permissions, so on macOS `prompt()` returns `null`, `confirm()` returns `false`, and `alert()` does nothing — every action gated on one is a **silent no-op**, with no error anywhere. This shipped: abandon, discard, delete-bookmark, rebase and create-bookmark were all dead in the app while working fine in `pnpm dev`.

Use `askText` / `askConfirm` from `ui/src/prompt.ts` instead. They also work in the browser mock, which the native dialog plugin does not.

**An overlay's backdrop test is `event.composedPath()[0] === this`, never `event.target === this`.** The scrim is the host element and the listener is bound to the host, so an event from inside the shadow root is *retargeted to the host* and a click on the panel is indistinguishable from a click on the scrim. All five overlays (command bar, prompt, theme picker, shortcuts sheet, evolog drawer) had this and dismissed on any click they received — the theme picker's filter box could not be clicked, and the version radios did nothing.

Same class of gap: **`target="_blank"` does nothing** — there is no tab to open. Outbound links go through the `open_url` command (`editor::open_url`), which hands the URL to the OS and refuses any scheme that isn't http/https.

## Ahead/behind is reported inverted by jj

`Repo::bookmark_statuses` reads `tracking_ahead_count` / `tracking_behind_count`, and **both mean the opposite of what the field names in `BookmarkStatus` mean**. The keywords live on the *remote* ref and describe the remote's position — `jj bookmark list` prints `@origin (behind by 2 commits)` when your local branch is two commits **ahead**. jjdiff states everything from the local bookmark's side (`ahead` = a push would send these), so the mapping is crossed exactly once, in that one function. Read it straight and the badges are backwards in a way that looks entirely plausible; `bookmark_statuses_report_ahead_and_behind_from_the_local_side` drives both directions against a real remote for that reason.

The synthetic `git` remote of a colocated repo is filtered out — it is in sync by construction, so reporting it is noise.

The one place this matters beyond a badge is the PR banner: `renderHeadDrift` warns when the proposal's head branch has unpushed commits, because CI, reviewers and merge state all describe the head *the forge has*. A green "checks passed" beside unpushed work reads as approval of code the forge never saw.

## Rewriting immutable commits is per-call, never a mode

`Repo::allowing_immutable(true)` returns a handle whose **rewriting** verbs carry `--ignore-immutable`; `Repo::mutate_rewriting` applies it and `Repo::mutate` does not. The split is not cosmetic: `backout` and `duplicate` *reject* the flag (they add commits rather than rewrite them), so funnelling every mutation through one helper turns an unrelated command into a clap parse error.

The opt-in is a per-call argument all the way from `ui/src/ipc.ts` to the CLI — nothing stores it. jj marks commits immutable to stop them being rewritten by accident, and a persisted "allow immutable" toggle would hand that guarantee back for a whole session instead of one confirmed command. On the frontend every such action routes through `App.confirmImmutableRewrite`, which returns `true` immediately for mutable changes and otherwise names the bookmark, the force-push, and the descendants that get rebased. Adding a new rewriting command means threading `allowImmutable` through and calling that helper — a rewriting action that skips it is one that silently rewrites published history.

## Keyboard shortcuts

Handlers live in `App.onGlobalKey` and `PatchView`; the `?` sheet renders `shortcutReference()` from `keys.ts`. That table is documentation, not dispatch, so a new binding needs both — a shortcut with no entry is one nobody can discover.
