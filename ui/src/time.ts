/**
 * Relative age, in the two forms the UI needs: compact ("3h") for dense rows like the
 * log graph, and suffixed ("3h ago") for prose-adjacent lines like the operation log.
 *
 * One implementation rather than one per view: the copies had drifted, and the one in
 * the app shell capped at days, so an operation from last month read "412d ago".
 */
export function relativeTime(timestamp: string, suffix = false): string {
  const compact = compactAge(timestamp);
  if (!compact || compact === 'now') return compact;
  return suffix ? `${compact} ago` : compact;
}

function compactAge(timestamp: string): string {
  const then = Date.parse(timestamp);
  if (Number.isNaN(then)) return '';
  const seconds = Math.max(0, (Date.now() - then) / 1000);
  if (seconds < 60) return 'now';
  const minutes = seconds / 60;
  if (minutes < 60) return `${Math.floor(minutes)}m`;
  const hours = minutes / 60;
  if (hours < 24) return `${Math.floor(hours)}h`;
  const days = hours / 24;
  if (days < 7) return `${Math.floor(days)}d`;
  const weeks = days / 7;
  if (weeks < 9) return `${Math.floor(weeks)}w`;
  return `${Math.floor(days / 30)}mo`;
}
