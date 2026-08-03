/**
 * What is here and not on a remote.
 *
 * Two questions that look like one and are not, which is why both halves live
 * together:
 *
 * - A **bookmark** can be ahead of the remote it tracks. `BookmarkStatus` says
 *   by how much, and jjdiff states it from the local side (see
 *   `Repo::bookmark_statuses` — the jj keywords it comes from mean the inverse).
 * - A **change** can be unpushed with no bookmark at all. It tracks nothing, so
 *   it has no ahead count and appears in no `BookmarkStatus` however long it
 *   sits there. `RepoState.unpushed` is that half.
 *
 * Only the logic is shared. The badge itself is drawn separately by the graph
 * and by the detail card, because one is inside a shadow root with its own
 * styles and the other is light DOM on `theme.css` — a shared `TemplateResult`
 * would need its classes defined in both places anyway.
 */

import type { BookmarkStatus, Change } from './ipc.js';

/**
 * How `bookmark` stands against the remote worth reporting.
 *
 * A bookmark may track several remotes. The one that has *drifted* is the one
 * worth a badge, so a diverged remote beats a synced one rather than whichever
 * jj happened to list first. `null` when the bookmark tracks nothing — a purely
 * local bookmark has no remote to be ahead of, which is not the same as being
 * in sync with one.
 */
export function worstTracking(
  statuses: readonly BookmarkStatus[],
  bookmark: string,
): BookmarkStatus | null {
  const all = statuses.filter((entry) => entry.name === bookmark);
  return all.find((entry) => entry.ahead || entry.behind) ?? all[0] ?? null;
}

/**
 * Whether this change is unpushed work that *nothing else on the row says so
 * about*.
 *
 * A change whose bookmark already shows `↑2` is unpushed and is already marked;
 * a second marker beside it would be the same fact twice. What this catches is
 * work no tracking badge can reach — a change with no bookmark, or one carrying
 * only a local bookmark that tracks no remote.
 */
export function unpushedAndUnnamed(
  change: Change,
  unpushed: ReadonlySet<string>,
  statuses: readonly BookmarkStatus[],
): boolean {
  // By commit id. The set holds commits, because "is this on a remote" is a
  // question about one snapshot and a divergent change has several — asked by
  // change id, publishing either one answered for both, and the graph put
  // "never been pushed" on a commit that was.
  if (!unpushed.has(change.commitId)) return false;
  return !change.bookmarks.some((bookmark) => worstTracking(statuses, bookmark)?.ahead);
}
