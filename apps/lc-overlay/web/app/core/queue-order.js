// Pending-queue ordering: the overlay shows the server order and lets the
// moderator drag a media onto another row (or move it one step with the
// keyboard). The new order is sent whole, so these helpers always return a
// complete list, never a partial edit.

// Returns a new list where `jobId` moved by `delta` places (`-1` up, `+1` down).
// Unknown ids, out-of-range moves and `delta === 0` leave the list unchanged.
export function moveInQueue(jobs, jobId, delta) {
  const index = jobs.findIndex((job) => job.id === jobId);
  const target = index + delta;
  if (index < 0 || delta === 0 || !Number.isInteger(delta) || target < 0 || target >= jobs.length) {
    return jobs.slice();
  }
  const next = jobs.slice();
  const [moved] = next.splice(index, 1);
  next.splice(target, 0, moved);
  return next;
}

// Returns a new list where `jobId` takes the place of `targetId`; the rows
// between the two slide around it. Unknown ids or a self-drop return the list
// unchanged.
export function moveToQueue(jobs, jobId, targetId) {
  const from = jobs.findIndex((job) => job.id === jobId);
  const to = jobs.findIndex((job) => job.id === targetId);
  if (from < 0 || to < 0 || from === to) {
    return jobs.slice();
  }
  const next = jobs.slice();
  const [moved] = next.splice(from, 1);
  next.splice(Math.min(to, next.length), 0, moved);
  return next;
}

// Ids of the pending jobs, in the order the server must persist.
export function queueJobIds(jobs) {
  return jobs.map((job) => job.id);
}
