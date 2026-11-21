// Thin activity bar at the top of the window.
//
// Fast local calls (the normal case) never show it: it appears only when a
// request has been running for 250 ms, so a slow server is visible without a
// spinner on every click.

const DELAY_MS = 250;
let count = 0;
let timer = null;
let bar = null;

export function beginActivity() {
  count += 1;
  if (count !== 1) {
    return;
  }
  timer = setTimeout(() => {
    timer = null;
    if (count > 0) {
      ensureBar();
      if (bar) {
        bar.hidden = false;
      }
    }
  }, DELAY_MS);
}

export function endActivity() {
  count = Math.max(0, count - 1);
  if (count > 0) {
    return;
  }
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  if (bar) {
    bar.hidden = true;
  }
}

export function activityCount() {
  return count;
}

function ensureBar() {
  if (bar || typeof document === 'undefined') {
    return;
  }
  bar = document.createElement('div');
  bar.className = 'activity-bar';
  bar.setAttribute('role', 'progressbar');
  bar.setAttribute('aria-label', 'Synchronisation avec le serveur');
  bar.hidden = true;
  document.body.append(bar);
}
