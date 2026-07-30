---
name: jjdiff
description: Author a guided review walkthrough for the current jj change and open it in jjdiff. Use when the user asks to walk someone through a change, explain a diff, or hand off work for review.
---

# jjdiff walkthrough

Write a guided review for the change you just made, then open jjdiff on it. Because you
already hold the context of *why* the change looks the way it does, this produces a better
walkthrough than jjdiff regenerating one from the diff alone.

A walkthrough has two parts: an **overview document** the reviewer reads before any code,
and an ordered set of **steps** through the diff.

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
     "summary": "one plain paragraph, no markdown",
     "overview": "# Title\n\n…markdown document, see below…",
     "steps": [
       { "title": "short noun phrase", "narrative": "1-4 sentences", "hunkIds": ["src/a.rs#0"] }
     ]
   }
   ```

3. Open jjdiff on it:

   ```bash
   jjdiff --walkthrough-file /tmp/walkthrough.json [revset]
   ```

## The overview

`overview` is a markdown document describing the change as a whole. It is a synthetic
description, not a file-by-file summary of the diff, and it never prescribes an
implementation. Write only what the diff supports. jjdiff renders it as a full page, with
` ```mermaid ` fences drawn as diagrams.

Markers: ➕ addition · ✏️ modification · ➖ deletion.

Any section may have nothing to report. Write `None` under it; do not invent entries to
fill it.

Open with a `#` heading naming the change, one short paragraph stating its purpose, the
marker legend line, then these four sections in this order.

### Impacted Systems

A ` ```mermaid ` fence holding a `flowchart LR` of the concrete processes, services,
binaries, crates, modules or applications that a changed boundary connects. Quote every
node label and every edge label. Label an edge `existing` when it is unchanged and appears
only as context.

### Changes to System Boundaries

One `###` section per changed boundary, headed
`### <marker> <left system> ⇄ <right system> — <what crosses>`. A boundary is where two
systems meet: an IPC or RPC surface, a CLI one process shells out to, a wire or file
format, a database schema, a public API of one crate consumed by another, a protocol. Name
concrete systems, not a module-plus-caller pair. Do not list an unchanged downstream
boundary as changed merely because new routing now reaches it. Under each:

- **Routing** — bullets: which side handles what, and what a side does with a call it does
  not handle.
- **Files:** one to three changed source files, each as inline code holding its
  repository-relative path exactly as it appears in the diff, separated by ` · `. jjdiff
  turns those paths into links into the diff itself, so a path it cannot match is a dead
  link.
- **Contract changes** — a ` ```diff ` fence giving the relevant shapes and operations
  almost in full: inputs, outputs, variants, optional fields, collections and errors.
  Prefix an added declaration with `+` and a removed one with `-`; inside an otherwise
  unchanged shape, prefix only the added or removed fields. Leave necessary unchanged
  context unprefixed. Write them in the language of the code being changed.
- Any behaviour the shapes do not state and a reviewer would have to infer: error mapping,
  what a failure leaves behind, what is deliberately not forwarded.

### Changes to Mutable State

A markdown table headed `| State | Ownership, cardinality, lifecycle |`, one row per added,
modified or deleted piece of held data. Put the major system on the first line of the right
cell and the concrete owner — struct, closure, module variable, component, table, cache —
on the second. Cardinality describes the data relationship itself, not the number of
copies. Record held data only: not function bags, clients, handles or other resources that
own no data, and not existing state merely because new code reads it.

### Changes to Effects

A markdown table headed `| Effect | Ownership and failure handling |`, one row per changed
entry point that makes the system touch the outside world: filesystem, persistence,
network, OS, external process. A call across a boundary already listed above is not an
effect. If a changed entry point reaches a pre-existing effect, record the entry point and
name the existing downstream work rather than calling that work changed. Do not record
ordinary query, synchronization or cache work — record its state instead. Same two-line
ownership convention, and keep failure handling to the behaviour that changed.

## The steps

Order the steps so understanding builds: core change first, then its callers, then
tests/config/mechanical fallout. Group related hunks into one step.

Rules jjdiff enforces on import — violating them silently loses content:

- Every `hunkIds` entry must exist in the diff; invented ids are dropped.
- **All hunks of one file belong to one step.** Reviewers mark whole files viewed, so a
  file split across steps shows as already-seen in the later one.
- A step referencing no real hunk is dropped entirely.

The walkthrough is stored against the change id, so it survives `jj describe`/`squash` and
is flagged stale if the diff later moves.
