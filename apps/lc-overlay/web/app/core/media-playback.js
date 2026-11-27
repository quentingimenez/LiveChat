// How a preview or a details media starts: with or without playing, with or
// without sound. A video may autoplay silently, but an audio never starts
// muted: it waits until its sound setting is on, so autoplay alone is not
// enough for it.

export function playbackPlan({ kind, autoplay, sound } = {}) {
  const muted = sound !== true;
  return {
    autoplay: autoplay === true && (kind !== 'audio' || !muted),
    muted,
  };
}

// The two hover-preview switches only apply while the hover preview itself is
// on: without it there is no preview player to start or unmute.
export function previewPlaybackEnabled(settings) {
  return !settings || settings.hoverPreview !== false;
}
