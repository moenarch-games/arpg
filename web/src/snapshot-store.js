// Session-owned authoritative snapshots. Only tick-dependent views subscribe;
// the shell and renderer configuration stay outside that React update domain.
//
// `beginSource()` marks that a new authority source (session, restart, restore) will publish
// next, so tick-sequenced consumers such as presentation cues can tell a replaced timeline
// from a late duplicate. It does not notify: the next publish carries the new source.
export function createSnapshotStore() {
  let snapshot = null;
  let source = 0;
  const listeners = new Set();
  return {
    getSnapshot: () => snapshot,
    getSource: () => source,
    beginSource() {
      source += 1;
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    publish(next) {
      if (Object.is(snapshot, next)) return;
      snapshot = next;
      for (const listener of listeners) listener();
    },
  };
}
