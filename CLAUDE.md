# CLAUDE.md

`jjdiff` is a Tauri 2 + Rust + Lit app for reviewing Jujutsu changes in colocated repos.
Read `DESIGN.md` before UI work.

## Commands

```bash
pnpm install
pnpm tauri dev
pnpm tauri dev -- -- -R /path/to/repo
pnpm dev
pnpm build

cargo test --workspace
cargo test -p jjdiff-vcs log_and_stack_roundtrip
cargo clippy --workspace --all-targets -- -D warnings
```

Run `pnpm build` before any `cargo` command in a clean checkout.
There is no JS test runner; `pnpm build` is the frontend check.

## Map

Flow: `jj` CLI -> `crates/vcs` -> `crates/diff` -> Tauri command -> `ui/src/ipc.ts` -> Lit UI.

- `crates/vcs`: all jj subprocesses.
- `crates/diff`: git patch and worktree diff parsing.
- `crates/watch`: repo/worktree watchers.
- `src-tauri/src/lib.rs`: Tauri commands.
- `ui/src/app.ts`: shell and most state.
- `ui/src/ipc.ts`: frontend command mirror.
- `ui/src/patch-view.ts`: diff view.
- `ui/src/overlay.ts`: modal base class.

## Backend Rules

- jj/filesystem Tauri commands are `async` and use `blocking()`.
- Every jj mutation goes through `run_mutation`.
- Adding a command requires `#[tauri::command]`, `generate_handler![...]`, and `ui/src/ipc.ts`.
- Undo is `jj op revert <operation id>`, not `jj undo`.
- Reads use `--ignore-working-copy --color=never --no-pager`.
- Mutations do not use read flags.
- Structured jj output comes from `json()` templates.
- Repos must be colocated.
- Keep `Repo::discover` paths separate: workspace root, `.jj/repo`, git dir.
- Each window owns its repo through `WindowState`.
- Repo commands use `repo_handle(state, window)`.
- Review-state commands use `repo_key(state, window)`.
- Repo switches must reset per-repo UI state through `RepoScoped` / `freshRepoScope()`.

## Jujutsu Rules

- Review state keys by change id.
- jj command revsets use commit ids.
- Use `revisionOf(change)` before calling jj.
- Working-copy diff uses `null` to mean live filesystem.
- Divergent changes need commit ids; never pass their change id to jj.
- Selection is `(changeId, commitId)`.
- Resolve exact commit first, then fall back to change id after rewrites.
- Resolve divergence by abandoning unwanted commits in one union revset.
- Map bookmark ahead/behind once; jj phrases it from the remote side.
- Find unbookmarked unpushed work with `remote_bookmarks()..`.
- `--ignore-immutable` is per rewriting command only.
- Do not make immutable rewrite a mode or setting.

## Frontend Rules

- `jj-app` and `jj-patch-view` use light DOM.
- Do not put shadow DOM above diff rows.
- Gate all IPC with `IN_TAURI`.
- New IPC needs a `ui/src/mock.ts` path.
- Add commands to `App.commands`; the native menu mirrors it.
- Keep the native Edit menu for macOS copy/paste.
- Actions go through `App.act`.
- Use `command()` for jj mutations and `run()` for non-mutating work.
- `busy` is a label.
- Failures go through `report(error)`.
- Use `askText` / `askConfirm`; never browser `alert`, `confirm`, or `prompt`.
- Open external links through `open_url`; `_blank` is inert.
- Keep Tauri drag/drop disabled in config and window setup.
- Add every new overlay flag to `App.overlayOpen`.
- Overlay backdrop checks use `event.composedPath()[0] === this`.
- Text-field overlays bind keys on the panel.
- No margins on virtualized rows.
- Avoid backticks inside `css` or `html` template literals, including comments.
- Line span offsets are UTF-16 code units.
- Shortcut handlers and `keys.ts` must stay in sync.

## UI / Config

- `DESIGN.md` is binding for UI changes.
- Themes are derived in `ui/src/themes.ts`.
- Only light/dark are hand-written in `theme.css`.
- Add a theme in both the seed list and `THEME_LOADERS`.
- Config writes go through `config::set_setting` and `App.applySetting`.
- Config writes are allow-listed, typed, and use `toml_edit`.
- Do not serde round-trip the user's config.
- `[editor] command` is split before template substitution and run without a shell.

## Forge

- Forge support is GitHub via `gh`; jjdiff handles no tokens.
- Test parsers against real `gh` fixture output.
- Never diff a proposal with `base..head`.
- Use the forge merge base (`baseRefOid`).
- File content needs one revision via `App.contentRevset`.
- A proposal is context on a change; the proposal view is `viewMode === 'pr'`.
- The proposal index is one page of open proposals.
- Use `find_by_head` as the exact bookmark fallback.
- Cache branch lookups as promises; clear them when the index reloads.
- Creating a proposal is: set bookmark if needed, push, then `gh pr create`.
- Refresh proposal state on focus and after push.
- Render forge markdown through `ui/src/markdown.ts`.
- Do not rely on CSP under `pnpm dev`.

## Special Flows

- `jj split -i` and `jj squash -i` run jjdiff as jj's diff editor.
- The frontend builds hunk plans from the visible diff.
- The backend verifies context and full right-side reproduction before writing.
- Preserve CRLF only when every line on both sides has it.
- Refuse mixed line endings.
- Squash uses `SplitPlan::moves`, `--use-destination-message`, and immutability checks on both ends.
- Conflict navigation moves between marker regions, not files.
- Marker length is per conflict region.
- Reject conflict resolutions that still contain fences.
- Keep walkthrough guide text synced across `walkthrough::GUIDE`, `cli::WALKTHROUGH_GUIDE`, and `skills/jjdiff/SKILL.md`.
- Walkthrough markdown and diagrams are untrusted; sanitize them.
- Generated commit messages are drafts.
- `generate_description` returns text only; it must not describe the change.

## Tests

- `crates/vcs` and `crates/diff` tests shell out to real `jj`.
- They skip when jj is missing and serialize through `JJ_LOCK`.
- New jj-backed tests set `signing.behavior=drop`, `JJ_USER`, and `JJ_EMAIL`.
- Check worktree-walk changes against real `jj status`.
