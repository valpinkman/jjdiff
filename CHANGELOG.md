# Changelog

All notable changes to jjdiff are documented here.

## [0.1.0] - 2026-08-09


### ✨ Features

- :sparkles: (forge): Open a pull request, and see what has not left the machine ([3197ec6](https://github.com/valpinkman/jjdiff/commit/3197ec6b7d93e6402adc0bffccc9652bcc01e640))

- :sparkles: (bookmark): add move bookmark action ([f853aad](https://github.com/valpinkman/jjdiff/commit/f853aad61d3faf656efa95d47f6521740a5779e4))

- :sparkles: (log): add full-page graph view ([fa20740](https://github.com/valpinkman/jjdiff/commit/fa20740ae9c6288437e449b82487906a7459d453))


### 🐛 Bug Fixes

- :bug: (vcs): Report unpushed work by commit id, not change id ([ee1f63f](https://github.com/valpinkman/jjdiff/commit/ee1f63fd78dd59204a37a116a6de48ef547d1aeb))

- :bug: (log-graph): Fix bookmark drag-and-drop broken by nested draggables, and other polish ([e189730](https://github.com/valpinkman/jjdiff/commit/e189730ec74850ec79ed7259184d3d3a83a7eba3))


### 💄 UI & Polish

- :lipstick: (detail): Toolbar above full-width prose ([abb38fa](https://github.com/valpinkman/jjdiff/commit/abb38faddb448f82a1516b2788de261b1f0583d2))

- :lipstick: (theme): A card is marked on its border, not filled ([e8a7acb](https://github.com/valpinkman/jjdiff/commit/e8a7acbeefca517ff70d4fb58737b7c3fbe439b5))

- :lipstick: (detail): Use flex layout for detail card sizing ([6553162](https://github.com/valpinkman/jjdiff/commit/6553162d5d7106e2177912b87f78c98e466ff38c))

- :lipstick: (cards): Toasts in the corner, one toolbar, and a revset that resolves ([33291ab](https://github.com/valpinkman/jjdiff/commit/33291abce073ad5271f35b3b663d93d857d26575))


### 📝 Documentation

- :memo: (claude): Slim the guidance file to its rules ([8f0f954](https://github.com/valpinkman/jjdiff/commit/8f0f95490906a94249e6bfcb644ed77368dad0ec))

- :memo: (claude): condense agent guidance ([42aa143](https://github.com/valpinkman/jjdiff/commit/42aa143bb9aa50393d95142b10ee97d9198ad105))


### 👷 CI

- :construction_worker: (ci): Add signed release workflow, pin actions to SHAs ([429c372](https://github.com/valpinkman/jjdiff/commit/429c372574a6444e217ca7eefa83fd5af6a0793d))


### Other Changes

- Plan jj-native diff review tool (Tauri + Rust) ([f1bbe5a](https://github.com/valpinkman/jjdiff/commit/f1bbe5aac1e4c795b25257abe60fdc8e0742a49e))

- Switch frontend plan to Lit 3 (light-DOM code pane, CSS custom property theming) ([8bc8082](https://github.com/valpinkman/jjdiff/commit/8bc80827823ad4678e7d24f8639413d3dd0c9333))

- M0: Tauri+Lit skeleton — jj facade, patch parser, op-head watcher, IPC, minimal UI ([671d852](https://github.com/valpinkman/jjdiff/commit/671d852e11a44c2307ad60dde2fa52070616de75))

- M1: virtualized diff viewer — fs-vs-tree working-copy diffs, split/unified, word spans, file tree, shiki ([98e0948](https://github.com/valpinkman/jjdiff/commit/98e0948ad5a68e425a8f421d3f25dc54cb77f09b))

- M2: daily-driver actions — fs watcher, describe/commit flow, squash, viewed flags, command bar, config ([3363026](https://github.com/valpinkman/jjdiff/commit/3363026c496515a16b951ea6a5e3535cf58dff61))

- M3: stack review — evolog interdiffs, mark-reviewed, absorb, move-to-change, conflict surfacing ([2119880](https://github.com/valpinkman/jjdiff/commit/211988067a6c5cb8684cc9a22a4e7724b3d9c4c4))

- M4: polish + packaging — themes, keymap, window title, release bundles ([122e8c5](https://github.com/valpinkman/jjdiff/commit/122e8c51d094e3f1f7f1a4227d3dce46abe618e5))

- Fix diff pane styling: light-DOM app shell + virtualize directive, flat rows, browser mock ([27f992e](https://github.com/valpinkman/jjdiff/commit/27f992e3c761f3e4118d40cdfb41809619057bc2))

- M5: guided review walkthroughs — Claude backend, change-id keyed, staleness-aware ([6353c24](https://github.com/valpinkman/jjdiff/commit/6353c24ebca44cbfcf13f289243059a9da2a9586))

- Walkthrough sidebar: Linear-style step list with per-step files + viewed progress ([334daab](https://github.com/valpinkman/jjdiff/commit/334daab56eb7edb8645b0d5aac77930c8814ba91))

- Collapse viewed files during guided review too ([24d1730](https://github.com/valpinkman/jjdiff/commit/24d173068814b3f1a8b038e7612db15a4d37b9ce))

- Tabbed sidebar + VS Code-style file tree ([3e80097](https://github.com/valpinkman/jjdiff/commit/3e8009724f09c8b809290b95ba12a8fa76e063c6))

- Redesign pass (redesign-skill) + DESIGN.md system spec (stitch-skill) ([48ddd1b](https://github.com/valpinkman/jjdiff/commit/48ddd1bf9944cc4216a9e41dcd6de58e70501bef))

- Brutalist variant (brutalist-skill, tempered): print/telemetry substrates ([b3736ad](https://github.com/valpinkman/jjdiff/commit/b3736adea5b27a5ec49506b44922cf6112309f18))

- Guided review refinements: file-exclusive steps, refresh polish, stack review ([01289ab](https://github.com/valpinkman/jjdiff/commit/01289ab8e08ca008fe80e75b91c3ac548f63a89f))

- Log tab: commit graph with lanes and branch rails ([030d51a](https://github.com/valpinkman/jjdiff/commit/030d51ac69132e6d91aa36e7b71b311538fdb9ab))

- Header rework: repo identity + switcher, version removed ([fdef8a2](https://github.com/valpinkman/jjdiff/commit/fdef8a20f470a788f571b002c16b1c04ea2ee7c6))

- CI: add tangled.sh pipeline for checks ([abc226c](https://github.com/valpinkman/jjdiff/commit/abc226c86dec5e7d49a311d45b44112828c073c2))

- CI: flatten dependencies list for microVM engine ([9e7627d](https://github.com/valpinkman/jjdiff/commit/9e7627d11c30c9c6076879074c225d91cc82625b))

- bug: (ci): Fix microVM eval failure, pnpm 11, split workflows ([89936a6](https://github.com/valpinkman/jjdiff/commit/89936a64c66b38f06377df0e5adba87e6857ddc3))

- Keyboard-first review + find-in-diffs ([e536055](https://github.com/valpinkman/jjdiff/commit/e536055aeb880047f80e85011832e0583f111fdc))

- Agent backends (codex/opencode/pi) + jj version gate, word wrap, launch revset ([6610eba](https://github.com/valpinkman/jjdiff/commit/6610eba62c192edd3f7a03230fe9fcd7f08c6f34))

- Diff navigation: tree-click scrolls, sticky file breadcrumb ([0f4b55b](https://github.com/valpinkman/jjdiff/commit/0f4b55bfc498d2ac6dbf474aed9ac187489c2c5d))

- Worktree rename detection + nested .gitignore in the fs watcher ([1da27fb](https://github.com/valpinkman/jjdiff/commit/1da27fbb6a60687ccd38d572256c9626b1383b60))

- Expand diff context beyond the hunks ([5681401](https://github.com/valpinkman/jjdiff/commit/5681401cc1f998ff62fedae120f63928eea25486))

- Agent-authored walkthroughs + plan close-out ([459309e](https://github.com/valpinkman/jjdiff/commit/459309ebf87337840bdee6b104dbbf66fa231602))

- Plan Phase 2: jj as a whole tool (detail view, ops, op log, revsets) ([4e9117d](https://github.com/valpinkman/jjdiff/commit/4e9117de96668e3993982927723e8a0a7b1213c8))

- Phase 2: change detail view, jj command surface, op log, revset filtering ([cbc0b23](https://github.com/valpinkman/jjdiff/commit/cbc0b23719875cbb3969ac7599456142d437b812))

- PLAN: mark Phase 2 shipped, record what is still open ([880733b](https://github.com/valpinkman/jjdiff/commit/880733bcc997e35f07d3cd2179c96463ed1b4860))

- Detail-view action tooltips + grouping ([9e4a146](https://github.com/valpinkman/jjdiff/commit/9e4a146e88577e5d4d1574bc7e98f956f6b7dc99))

- Grouped command palette, runtime theme switcher, slim header ([c3e3ea4](https://github.com/valpinkman/jjdiff/commit/c3e3ea43c69c212a3c03453c41258e7752659027))

- Collapsible change-detail block ([a3eb0d3](https://github.com/valpinkman/jjdiff/commit/a3eb0d3a32edad7ac7f59f14b9c02fd4d71310ca))

- Fix: the detail collapse control was a 10px chevron ([1f846f2](https://github.com/valpinkman/jjdiff/commit/1f846f21bfa401c01b6852306e3ceb76ac2faa9f))

- Redesign: modern layered look, refined diff cards, warmer sidebar ([f01870e](https://github.com/valpinkman/jjdiff/commit/f01870e557db2d8652fb34aa1d4e3c7261b66307))

- Close file cards; make repo-creating tests hermetic ([2016bb4](https://github.com/valpinkman/jjdiff/commit/2016bb40e21e435a8f050102ed71e6739949dc01))

- PLAN: Phase 3 — CLI/terminal helper, review comments, images+markdown, forge review, native polish ([dda1f94](https://github.com/valpinkman/jjdiff/commit/dda1f9404a9beb703c9415e2db173c66b873d6bb))

- Add GitHub Actions CI, remove Tangled CI ([050fb41](https://github.com/valpinkman/jjdiff/commit/050fb4110a8de450c8168b273789cd25ccebf914))

- Merge pull request #1 from valpinkman/github-ci-workflows ([5960ed9](https://github.com/valpinkman/jjdiff/commit/5960ed995b0de7dba20b5a4f4fecebd94365fbb4))

- C1: CLI and terminal helper ([343c782](https://github.com/valpinkman/jjdiff/commit/343c782ddd0778b2eef7f4a33cfae08f6bb48616))

- Merge pull request #2 from valpinkman/valpinkman/jjdiff-cli-terminal-helper ([ff6422b](https://github.com/valpinkman/jjdiff/commit/ff6422b50826e66233461a5077ee5823ba356bd2))

- C2: Inline review comments (#3) ([b262223](https://github.com/valpinkman/jjdiff/commit/b26222343117d49d65ee7a9222c924c702b7ed64))

- C3: Image and markdown rendering ([70c32ee](https://github.com/valpinkman/jjdiff/commit/70c32eeb155a5142074d34d860691ea9756b4522))

- Merge pull request #4 from valpinkman/valpinkman/jjdiff-images-markdown ([df2799e](https://github.com/valpinkman/jjdiff/commit/df2799e69c566c7af1602982c1169eda31e3a604))

- UI refresh ([ddf90b9](https://github.com/valpinkman/jjdiff/commit/ddf90b9b1b3c221c163a6cb5c37bc4c40e381418))

- UI: diff pane gutters, in-column panning, log colour, icon set ([bdb5a4b](https://github.com/valpinkman/jjdiff/commit/bdb5a4b0a7784758fabba61734ad4e42c3c45e11))

- C5: Native app polish ([b8d2196](https://github.com/valpinkman/jjdiff/commit/b8d2196aa7adf0d7b2a35e3c0240f08464bb9bdd))

- C4: Forge review via gh / glab ([977f3ff](https://github.com/valpinkman/jjdiff/commit/977f3ffe45b8c3609543a15a2410b4027c4128be))

- Merge pull request #5 from valpinkman/valpinkman/jjdiff-polish-forge-review ([487c240](https://github.com/valpinkman/jjdiff/commit/487c240d116805692463ed1a0b0cea349afd042d))

- Bookmark tracking state, and rewriting immutable commits ([7d6202a](https://github.com/valpinkman/jjdiff/commit/7d6202ac67a83dd9e2289383878c59c3c78b4cd1))

- The proposal becomes a view; the description becomes readable ([8ec2063](https://github.com/valpinkman/jjdiff/commit/8ec206318566025f7c1620d0a226238cefacf921))

- Merge pull request #6 from valpinkman/valpinkman/jjdiff-tracking-immutable ([77100b5](https://github.com/valpinkman/jjdiff/commit/77100b5a3f4fc9350bffd53d8bf8175ecfba2c91))

- Visual overhaul: flat surfaces, an icon rail, and named themes ([f349d5d](https://github.com/valpinkman/jjdiff/commit/f349d5dc1b7811c57909891ad6701696d339beb3))

- Merge pull request #7 from valpinkman/valpinkman/jjdiff-visual-overhaul ([cb3881b](https://github.com/valpinkman/jjdiff/commit/cb3881b75007dda90fd46a6471ee2bceb6a01b87))

- Operation diffs, an evolog drawer, and three bugs they exposed ([ebcf9c6](https://github.com/valpinkman/jjdiff/commit/ebcf9c62230dc27ef85be75ebf720914153713e7))

- Hunk-level split, a rebase picker, and conflicts you can walk ([40d6ba1](https://github.com/valpinkman/jjdiff/commit/40d6ba17dd6f28035669fceb093e22ae3a1089e6))

- A proposal is a thing you review, not a change you happen to have selected ([b800307](https://github.com/valpinkman/jjdiff/commit/b8003072c414376d9c16937687a18fdd22d5e3ea))

- New on top, on the working copy too ([aeafd31](https://github.com/valpinkman/jjdiff/commit/aeafd315ffee4f6af4dc5970eb977cf582c63f00))

- The walkthrough overview becomes a document ([1988781](https://github.com/valpinkman/jjdiff/commit/19887819a40c58c3b96694e89868da47e2c7cc9c))

- Settings page and commit message generation ([d0ac794](https://github.com/valpinkman/jjdiff/commit/d0ac794d1f4697f6fbb93a166ea24e51318349f0))

- Rules that were only prose now have something behind them ([d0469e6](https://github.com/valpinkman/jjdiff/commit/d0469e62879b7e3a6cda3c2d0ddbf35f1654cc60))

- Hunk-level squash, conflicts you can settle, and rebase by drag ([6c9ccf8](https://github.com/valpinkman/jjdiff/commit/6c9ccf8a6f1f9de4e18bacdc9e5e57bbd42eaf7c))

- Workspaces, and the three paths a `Repo` was pretending were one ([6991c86](https://github.com/valpinkman/jjdiff/commit/6991c86ca521429a333f487795795435b2fcfd07))

- A menu on the graph, and three things the workspace work left crooked ([a38079e](https://github.com/valpinkman/jjdiff/commit/a38079ed2e8bfaf22ece090422e3ed87c86956b1))

- Surface change divergence, and disable Tauri's native drag/drop ([2775629](https://github.com/valpinkman/jjdiff/commit/2775629ca27b94d21651a3badafcd7542a7f8cf2))


