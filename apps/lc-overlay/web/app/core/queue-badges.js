// Which playback badge a board card shows.
//
// The queue is the source of truth: a job names the media asset it plays, and a
// board item names the asset it holds (one item per asset and guild). Matching
// both keeps the badge when the board is painted again — after a guild switch
// or a reload — without replaying the click that started the media. The click's
// answer (`triggers`/`hints`) only covers the moment between that click and the
// next queue read.

/** Jobs of this board item triggered from this session (`jobId -> itemId`). */
function jobIdsOf(triggers, itemId) {
  const ids = [];
  for (const [jobId, mapped] of triggers || []) {
    if (mapped === itemId) {
      ids.push(jobId);
    }
  }
  return ids;
}

/** The asset a queue job plays (the queue names it at the top level and in `media`). */
function assetOf(job) {
  return job?.mediaAssetId || job?.media?.id || null;
}

function matches(job, assetId, jobIds) {
  return Boolean(job) && ((assetId !== null && assetOf(job) === assetId) || jobIds.includes(job.id));
}

function pendingBadge(index) {
  return { air: false, label: `En file · ${index + 1}` };
}

/**
 * Badge of one board item: the job on air, else its pending position, else the
 * click's answer while no queue read has covered it yet.
 *
 * @param {{playing: object|null, pending: object[], at?: number}|null} queue
 * @param {Map<string, string>} triggers jobId -> itemId of the session's clicks
 * @param {Map<string, {outcome: string, position?: number|null, at: number}>} hints
 * @param {{id: string, mediaAssetId?: string|null}|null} item board item to badge
 * @returns {{air: boolean, label: string}|null}
 */
export function badgeForItem(queue, triggers, hints, item) {
  if (!item || !item.id) {
    return null;
  }
  const itemId = item.id;
  const assetId = item.mediaAssetId || null;
  const jobIds = jobIdsOf(triggers, itemId);
  if (matches(queue && queue.playing, assetId, jobIds)) {
    return { air: true, label: 'À l’écran' };
  }
  const pending = (queue && queue.pending) || [];
  const index = pending.findIndex((job) => matches(job, assetId, jobIds));
  if (index !== -1) {
    return pendingBadge(index);
  }
  const queueAt = (queue && queue.at) || 0;
  let queued = null;
  for (const jobId of jobIds) {
    const hint = hints && hints.get(jobId);
    // A queue read newer than the click has decided: the job is gone.
    if (!hint || queueAt > hint.at) {
      continue;
    }
    if (hint.outcome === 'playing') {
      return { air: true, label: 'À l’écran' };
    }
    if (hint.outcome === 'queued') {
      queued = queued || { air: false, label: hint.position ? `En file · ${hint.position}` : 'En file' };
    }
  }
  return queued;
}
