// The hover preview's progress bar.
//
// `timeupdate` fires about four times a second. Painting the fill there moves
// it in 250 ms steps, and the 90 ms transition that used to follow each step
// left the bar frozen for the rest of it: that is the stutter. The fill is
// instead a linear animation of the time left, run by the compositor at the
// display rate; the media clock only says where to (re)start it, on play,
// pause, seek, stall, drift and each loop.

export const PROGRESS_DRIFT = 0.25;

// Ratio of the media already played, inside 0..1. A duration the element has
// not learnt yet (NaN, Infinity) or a position outside the media reads as an
// empty bar rather than a NaN transform.
export function progressRatio(currentTime, duration) {
  if (!Number.isFinite(currentTime) || !Number.isFinite(duration) || duration <= 0) {
    return 0;
  }
  return Math.min(1, Math.max(0, currentTime / duration));
}

// Keyframes and timing for the fill, or null when it must not move: paused,
// stalled, no duration yet, or already at the end. `playing` is the caller's
// reading of the player, so a seeking or stalled element never animates.
// `playbackRate` keeps the fill on the media's wall clock: at 2x it crosses the
// same span in half the time. `startRatio` overrides the first keyframe when a
// pause froze the fill between two media frames: resuming from that displayed
// position avoids the small jump back to `currentTime`.
export function previewProgressFrames({
  currentTime,
  duration,
  playing = false,
  playbackRate = 1,
  startRatio,
} = {}) {
  if (playing !== true) {
    return null;
  }
  const ratio = progressRatio(currentTime, duration);
  const remaining = duration - currentTime;
  if (!Number.isFinite(remaining) || remaining <= 0) {
    return null;
  }
  const start = Number.isFinite(startRatio)
    ? Math.min(1, Math.max(0, startRatio))
    : ratio;
  const rate = Number.isFinite(playbackRate) && playbackRate > 0 ? playbackRate : 1;
  return {
    ratio: start,
    frames: [
      { transform: `scaleX(${start})` },
      { transform: 'scaleX(1)' },
    ],
    options: {
      duration: (remaining / rate) * 1000,
      easing: 'linear',
      fill: 'forwards',
    },
  };
}

// Drive one fill element from one media element. Returns the immediate paint
// and an idempotent destroy; the caller owns the nodes and the destroy call
// (the preview does it before pausing its player on close).
//
// State is read from the player rather than mirrored: `paused`, `seeking` and
// the `waiting`/`playing` pair are enough. `timeupdate` is a guard, not a
// clock: it only restarts the animation when the compositor and the media have
// drifted apart by more than PROGRESS_DRIFT seconds, or when the media jumped
// back (the loop), so a normal tick leaves the single running animation alone.
export function createPreviewProgress({ player, fill, drift = PROGRESS_DRIFT } = {}) {
  if (!player || !fill) {
    throw new TypeError('createPreviewProgress needs a player and a fill');
  }

  let animation = null;
  let startRatio = 0;
  let stalled = false;
  let resumeFrom = null;
  const listeners = new Map();

  const still = (ratio) => {
    fill.style.transform = `scaleX(${ratio})`;
  };

  const cancel = () => {
    const current = animation;
    animation = null;
    if (current) {
      current.cancel();
    }
  };

  // Where the compositor currently shows the fill, or null when there is no
  // running animation (or its progress is unavailable, e.g. in a hidden tab).
  const displayed = () => {
    if (!animation) {
      return null;
    }
    const timing = animation.effect && typeof animation.effect.getComputedTiming === 'function'
      ? animation.effect.getComputedTiming()
      : null;
    const progress = timing ? timing.progress : null;
    if (!Number.isFinite(progress)) {
      return null;
    }
    return Math.min(1, Math.max(0, startRatio + (1 - startRatio) * progress));
  };

  // `from` restarts the fill at a displayed ratio (a pause froze it between two
  // media frames); without it the fill follows `currentTime`.
  const paint = (from = null) => {
    resumeFrom = null;
    cancel();
    const mediaRatio = progressRatio(player.currentTime, player.duration);
    const start = Number.isFinite(from)
      ? Math.min(1, Math.max(0, from))
      : mediaRatio;
    still(start);
    const plan = previewProgressFrames({
      currentTime: player.currentTime,
      duration: player.duration,
      playing: !player.paused && !player.seeking && !stalled,
      playbackRate: player.playbackRate,
      startRatio: start,
    });
    if (!plan || typeof fill.animate !== 'function') {
      return;
    }
    startRatio = plan.ratio;
    const running = fill.animate(plan.frames, plan.options);
    animation = running;
    // The animation ends where the media ends. At a loop the media has already
    // gone back to zero (or is about to): restarting from the media clock puts
    // the fill back on the first frame instead of leaving a full bar until the
    // next `timeupdate`.
    running.onfinish = () => {
      if (animation !== running) {
        return;
      }
      paint();
    };
  };

  // The media clock and the display clock do not stop on the same frame:
  // `currentTime` on pause is the last decoded frame, while the compositor has
  // already shown the next one. Freezing at the displayed ratio avoids the
  // small backward jump; `currentTime` stays the fallback when it is unknown.
  const freeze = () => {
    const shown = displayed();
    cancel();
    if (shown !== null) {
      resumeFrom = shown;
      still(shown);
    } else if (resumeFrom === null) {
      still(progressRatio(player.currentTime, player.duration));
    }
  };

  const resume = () => {
    const from = resumeFrom;
    resumeFrom = null;
    paint(from);
  };

  const sync = () => {
    // The pause, stall and seek handlers already froze the fill; a late
    // `timeupdate` must not pull it back to the media clock.
    if (player.paused || player.seeking || stalled) {
      return;
    }
    if (!animation) {
      paint();
      return;
    }
    const timing = animation.effect && typeof animation.effect.getComputedTiming === 'function'
      ? animation.effect.getComputedTiming()
      : null;
    const progress = timing ? timing.progress : null;
    if (!Number.isFinite(progress)) {
      paint();
      return;
    }
    const shown = startRatio + (1 - startRatio) * progress;
    const actual = progressRatio(player.currentTime, player.duration);
    if (Math.abs(shown - actual) * player.duration > drift) {
      paint();
    }
  };

  const listen = (type, handler) => {
    player.addEventListener(type, handler);
    listeners.set(type, handler);
  };

  listen('play', resume);
  listen('playing', () => {
    stalled = false;
    if (resumeFrom !== null) {
      resume();
    } else if (!animation) {
      paint();
    }
  });
  listen('pause', () => {
    // A paused media is not stalled: the next play resumes from the frozen
    // ratio instead of waiting for a `playing` event.
    stalled = false;
    freeze();
  });
  listen('waiting', () => {
    stalled = true;
    freeze();
  });
  listen('stalled', () => {
    stalled = true;
    freeze();
  });
  listen('seeking', () => paint());
  listen('seeked', () => paint());
  listen('durationchange', () => paint());
  listen('ratechange', () => paint());
  listen('emptied', () => paint());
  listen('timeupdate', sync);

  paint();

  return {
    paint,
    destroy() {
      cancel();
      for (const [type, handler] of listeners) {
        player.removeEventListener(type, handler);
      }
      listeners.clear();
    },
  };
}
