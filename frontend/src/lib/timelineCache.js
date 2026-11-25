/**
 * Deduplicating loader for a value two components need at once: the timeline
 * slider and the date scrubber both read `/api/photos/timeline`, and both
 * refetch on `photosReloadRequested`.
 *
 * The cache lives for the page load. A forced reload refetches after a save;
 * it joins only a forced read that is already in flight, never the initial
 * read that may have started before the save. A slower stale answer can
 * therefore not overwrite the newer cache entry.
 *
 * @param {(endpoint: string) => Promise<any>} request
 */
export function createTimelineLoader(request) {
  let cached = null;
  let inFlight = null;
  let generation = 0;

  return function loadTimeline(force = false) {
    if (inFlight && (!force || inFlight.forced)) return inFlight.promise;
    if (cached && !force) return Promise.resolve(cached);

    const requestGeneration = ++generation;
    const entry = { forced: force, promise: null };
    entry.promise = (async () => {
      try {
        const data = await request('/api/photos/timeline');
        if (requestGeneration === generation) cached = data;
        return data;
      } finally {
        if (inFlight === entry) inFlight = null;
      }
    })();
    inFlight = entry;
    return entry.promise;
  };
}
