// Lane assignment for a commit graph, jj-log style.
//
// Input must be in reverse-topological order (children before parents) — exactly what
// `jj log` emits. Each row gets a lane (column); rails connect commits to their parents
// across rows. The algorithm is the classic streaming one: an array of "expected commit
// ids" per lane, updated row by row.
import type { Change } from './ipc.js';

export interface GraphRow {
  change: Change;
  /** Column of this commit's dot. */
  lane: number;
  /** Lanes (other than `lane`) whose expectation was this commit — they merge into the dot. */
  joins: number[];
  /** Lanes forked out of this commit for parents beyond the first. */
  forks: number[];
  /** Lanes that pass straight through this row (still expecting a later commit). */
  through: number[];
  /** True when the first parent continues below (draw a tail under the dot). */
  continues: boolean;
  /** Width (in lanes) needed to draw this row. */
  width: number;
}

export function layoutGraph(changes: Change[]): GraphRow[] {
  const lanes: (string | null)[] = []; // expected commit id per lane
  const rows: GraphRow[] = [];

  const allocate = (expected: string): number => {
    const free = lanes.indexOf(null);
    if (free !== -1) {
      lanes[free] = expected;
      return free;
    }
    lanes.push(expected);
    return lanes.length - 1;
  };

  for (const change of changes) {
    // The dot's lane: the first lane expecting this commit, else a new head lane.
    let lane = lanes.findIndex((expected) => expected === change.commitId);
    if (lane === -1) {
      lane = allocate(change.commitId);
    }
    const joins: number[] = [];
    lanes.forEach((expected, index) => {
      if (index !== lane && expected === change.commitId) {
        joins.push(index);
        lanes[index] = null;
      }
    });

    const [firstParent, ...extraParents] = change.parents;
    const rootParent = (id: string | undefined) => !id || /^0+$/.test(id);
    lanes[lane] = rootParent(firstParent) ? null : firstParent!;

    const forks: number[] = [];
    for (const parent of extraParents) {
      if (rootParent(parent)) continue;
      // Reuse a lane already waiting for this parent (the fork joins it lower down);
      // otherwise open a new lane.
      const existing = lanes.findIndex((expected) => expected === parent);
      forks.push(existing !== -1 ? existing : allocate(parent));
    }

    const through: number[] = [];
    lanes.forEach((expected, index) => {
      if (expected !== null && index !== lane && !forks.includes(index)) {
        through.push(index);
      }
    });

    // Trim dead tail lanes so width stays tight.
    while (lanes.length > 0 && lanes[lanes.length - 1] === null) {
      lanes.pop();
    }

    rows.push({
      change,
      lane,
      joins,
      forks,
      through,
      continues: lanes[lane] !== null && lanes[lane] !== undefined,
      width: Math.max(lane + 1, lanes.length, ...joins.map((j) => j + 1)),
    });
  }
  return rows;
}
