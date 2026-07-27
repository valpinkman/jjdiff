---
name: jjdiff
description: Author a guided review walkthrough for the current jj change and open it in jjdiff. Use when the user asks to walk someone through a change, explain a diff, or hand off work for review.
---

# jjdiff walkthrough

Write a guided review for the change you just made, then open jjdiff on it. Because you
already hold the context of *why* the change looks the way it does, this produces a better
walkthrough than jjdiff regenerating one from the diff alone.

> **Need the full authoring guide?** Run `jjdiff --walkthrough-guide` — it prints this
> document to stdout, so you can fetch it without having the skill installed.

## Steps

1. Get the diff with stable hunk ids:

   ```bash
   jjdiff --print-hunks            # working copy
   jjdiff --print-hunks <revset>   # a specific change
   ```

   Each hunk is printed as `<path>#<index>` followed by its lines.

2. Write JSON to a temp file matching exactly:

   ```json
   {
     "summary": "one-paragraph overview of the change",
     "steps": [
       { "title": "short noun phrase", "narrative": "1-4 sentences", "hunkIds": ["src/a.rs#0"] }
     ]
   }
   ```

   Rules jjdiff enforces on import — violating them silently loses content:
   - Every `hunkIds` entry must exist in the diff; invented ids are dropped.
   - **All hunks of one file belong to one step.** Reviewers mark whole files viewed, so a
     file split across steps shows as already-seen in the later one.
   - Order steps so understanding builds: core change first, then its callers, then
     tests/config/mechanical fallout.

3. Open jjdiff on it:

   ```bash
   jjdiff --walkthrough-file /tmp/walkthrough.json [revset]
   ```

The walkthrough is stored against the change id, so it survives `jj describe`/`squash` and
is flagged stale if the diff later moves.
