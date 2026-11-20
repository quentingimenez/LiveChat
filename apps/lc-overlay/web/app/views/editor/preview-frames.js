// The overlay preview is a canvas: a video only shows there at its real frame
// rate if the canvas is painted again for every frame the media presents.
// `timeupdate` fires about four times a second, so a playback that follows it
// looks like it runs at 4 fps.
//
// `requestVideoFrameCallback` fires for every presented frame, even while the
// `<video>` is hidden behind the preview. Browsers without it keep the
// `timeupdate` repaint, and the editor's animation loop stands in for the rest.
export function createPreviewFramePump({ media, onFrame, isActive }) {
  const supported = typeof media.requestVideoFrameCallback === 'function';
  let handle = null;
  let running = false;

  function pump() {
    handle = null;
    if (!running || !isActive() || media.paused) {
      running = false;
      return;
    }
    onFrame();
    handle = media.requestVideoFrameCallback(pump);
  }

  function start() {
    if (!supported || running || media.paused || !isActive()) {
      return;
    }
    running = true;
    handle = media.requestVideoFrameCallback(pump);
  }

  function stop() {
    running = false;
    if (handle !== null) {
      if (typeof media.cancelVideoFrameCallback === 'function') {
        media.cancelVideoFrameCallback(handle);
      }
      handle = null;
    }
  }

  return { supported, start, stop };
}
