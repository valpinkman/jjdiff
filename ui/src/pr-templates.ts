import { html, nothing, type TemplateResult } from 'lit';

import type { BookmarkStatus, PullRequest } from './ipc.js';

/**
 * The proposal's identity row, shared by the banner and the proposal view.
 *
 * Plain functions, not a custom element: the banner sits directly above the
 * diff, and a shadow root anywhere above the diff pane severs theme.css from
 * it (CLAUDE.md, "Light DOM above the diff pane").
 *
 * What is shared is the *items*, not the row. Each call site keeps its own
 * wrapper and class — `.pr-meta` and `.pr-view-meta` are separate rules — and
 * they differ deliberately at the ends: the view appends a spacer and a link
 * out to the forge, the banner a scope chip and the review composer. Forcing
 * both through one function with flags would cost more than the two wrappers.
 */

/**
 * What the row needs from the shell. All of it is per-repo state the app owns,
 * so it is passed in rather than reached for: `tracking` reads the bookmark
 * list off the loaded repo, `noun` is whatever the discovered forge calls a
 * proposal, and the three verbs run mutations the app narrates.
 */
export interface PrMetaContext {
  noun: string;
  tracking(bookmark: string): BookmarkStatus | null;
  openCheck(url: string): void;
  fetch(): void;
  push(bookmark: string): void;
}

/**
 * Author, branches, size, and everything that qualifies how far the proposal
 * can be trusted: conflicts, checks, head drift, reviewers.
 */
export function prMetaItems(pr: PullRequest, ctx: PrMetaContext): TemplateResult {
  return html`<span>${pr.author}</span>
    <span class="pr-branches"><code>${pr.base}</code> ← <code>${pr.head}</code></span>
    ${pr.additions || pr.deletions
      ? html`<span class="pr-stat">
          <span class="plus">+${pr.additions}</span>
          <span class="minus">−${pr.deletions}</span>
        </span>`
      : nothing}
    ${pr.mergeable === 'CONFLICTING'
      ? html`<span class="pr-conflict" title="This ${ctx.noun} has conflicts with its base branch"
          >⚠ conflicts</span
        >`
      : nothing}
    ${checks(pr, ctx)} ${headDrift(pr, ctx)} ${reviewers(pr)}`;
}

/**
 * Reviewers as name chips, verdict first.
 *
 * Only outcomes take colour (DESIGN.md §2): approved and changes-requested are
 * verdicts, a reviewer who has commented or not yet looked is neither.
 */
function reviewers(pr: PullRequest) {
  if (pr.reviewers.length === 0) return nothing;
  return html`<span class="pr-reviewers">
    ${pr.reviewers.map(
      (reviewer) => html`<span
        class="tag muted ${reviewer.state === 'APPROVED'
          ? 'approved'
          : reviewer.state === 'CHANGES_REQUESTED'
            ? 'changes'
            : ''}"
        title=${reviewerTitle(reviewer.state)}
        >${reviewer.state === 'APPROVED'
          ? '✓ '
          : reviewer.state === 'CHANGES_REQUESTED'
            ? '✕ '
            : ''}${reviewer.name}</span
      >`,
    )}
  </span>`;
}

/**
 * The proposal's state as a glyph + word.
 *
 * Only *outcomes* take colour (DESIGN.md §2): merged succeeded, closed did not.
 * Open and draft are neutral, because they are a status rather than a verdict —
 * which is what leaves the coloured ones worth noticing. GitHub's purple for
 * merged would be a third hue, so it stays out.
 */
export function proposalState(pr: PullRequest) {
  const state = pr.draft ? 'draft' : pr.state.toLowerCase();
  const glyph = { merged: '✓', closed: '✕', draft: '◌', open: '●' }[state] ?? '●';
  return html`<span class="pr-state ${state}" title=${`${state} · ${pr.mergeable.toLowerCase()}`}>
    <span class="pr-state-glyph">${glyph}</span>${state}
  </span>`;
}

const reviewerTitle = (state: string) =>
  ({
    APPROVED: 'approved',
    CHANGES_REQUESTED: 'requested changes',
    COMMENTED: 'commented',
    REQUESTED: 'review requested',
  })[state] ?? state.toLowerCase().replace(/_/g, ' ');

/**
 * CI summary. Reads as one verdict, not a row of counters: what a reviewer
 * needs is "can I trust this build", and only the failures are worth naming.
 * Failed checks are clickable — a red name with no way to reach the log is
 * an invitation to go hunting in a browser.
 */
function checks(pr: PullRequest, ctx: PrMetaContext) {
  if (pr.checks.length === 0) return nothing;
  const failed = pr.checks.filter((check) => check.conclusion === 'FAILURE');
  const running = pr.checks.filter((check) => check.status !== 'COMPLETED');
  const passed = pr.checks.filter((check) => check.conclusion === 'SUCCESS');
  if (failed.length) {
    return html`<span class="pr-checks bad">
      <span>✕ ${failed.length} of ${pr.checks.length} failed</span>
      ${failed.map(
        (check) => html`<button
          class="pr-check-name"
          title="Open ${check.name} on the forge"
          @click=${() => ctx.openCheck(check.url)}
        >
          ${check.name}
        </button>`,
      )}
    </span>`;
  }
  if (running.length) {
    // Neutral, and pulsing rather than spinning (DESIGN.md §6): in progress
    // is not an outcome, so it must not read as one.
    return html`<span class="pr-checks pending">
      <span class="dot"></span>
      ${running.length} check${running.length === 1 ? '' : 's'} running
    </span>`;
  }
  if (passed.length) {
    return html`<span class="pr-checks ok"
      >✓ ${passed.length} check${passed.length === 1 ? '' : 's'} passed</span
    >`;
  }
  return nothing;
}

/**
 * How far the proposal's head branch has drifted from its remote, and the one
 * command that closes the gap.
 *
 * Drift in either direction makes everything beside it untrustworthy: the
 * checks, reviews and merge state all describe the head *the forge has*. So
 * the warning is not decoration, and neither is the button — being told your
 * work is unpushed and then having to go and find the push command is the
 * gap this closes.
 *
 * The verb follows the direction rather than being a single "sync": behind
 * wins when both are true, because a push against a moved remote is rejected
 * as a non-fast-forward, and doing both silently would rebase your work
 * without asking.
 */
function headDrift(pr: PullRequest, ctx: PrMetaContext) {
  const tracking = ctx.tracking(pr.head);
  const ahead = tracking?.ahead ?? 0;
  const behind = tracking?.behind ?? 0;
  if (!ahead && !behind) return nothing;
  const plural = (count: number) => (count === 1 ? '' : 's');
  return html`<span
      class="pr-drift"
      title=${
        behind
          ? `${pr.head}@remote has ${behind} commit${plural(behind)} this repo does not. Everything this ${ctx.noun} reports is about that head, not the code shown here.`
          : `${ahead} local commit${plural(ahead)} on ${pr.head} ${
              ahead === 1 ? 'has' : 'have'
            } not been pushed. Everything this ${ctx.noun} reports — checks, reviews, merge state — is about the head the forge has, not the code shown here.`
      }
      >⚠ ${behind ? `${behind} behind` : `${ahead} unpushed`}</span
    >
    <button
      class="tool pr-sync"
      title=${
        behind
          ? `jj git fetch — bring in the ${behind} commit${plural(behind)} the remote has. Integrate before pushing; a push over a moved remote is refused.`
          : `jj git push -b ${pr.head} — send the ${ahead} unpushed commit${plural(ahead)} so the ${ctx.noun} describes the code you are looking at.`
      }
      @click=${(event: Event) => {
        // The banner is itself a button into the proposal view.
        event.stopPropagation();
        if (behind) ctx.fetch();
        else ctx.push(pr.head);
      }}
    >
      ${behind ? 'Fetch' : 'Push'}
    </button>`;
}
