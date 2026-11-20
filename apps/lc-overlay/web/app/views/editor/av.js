// Audio and video editor: a timeline to trim and cut, speed, volume, fades,
// reverse, and for a video crop, rotation, flips and the output format.
//
// Nothing is encoded here. The editor previews the result with the media
// element (skipping what was removed, at the chosen speed and volume) and
// produces a recipe; the server applies it with ffmpeg.

import { h, icon, registerIcons, resetOnDoubleClick } from '../../core/dom.js';
import {
  CROP_RATIOS,
  DEFAULT_DISPLAY_SCALE_PERCENT,
  MAX_DISPLAY_SCALE_PERCENT,
  MIN_DISPLAY_SCALE_PERCENT,
  MIN_KEEP_SEC,
  MAX_SPEED,
  MIN_SPEED,
  OVERLAY_MEDIA_RESERVED_HEIGHT,
  SPEEDS,
  avEditProblem,
  buildRecipe,
  centeredCrop,
  clamp,
  cutRange,
  defaultAvEdit,
  displayScaleForStorage,
  displayZoneLimits,
  displayedImageSize,
  fadeGain,
  formatClock,
  formatSpeed,
  isWhole,
  keepRange,
  keptDuration,
  nextKeptTime,
  normalizeDisplayScale,
  normalizeSegments,
  outputDuration,
  parseClock,
  parseDisplayScale,
  parseSpeed,
  removedRanges,
  rotatedSize,
  showsDisplaySize,
  sourceToOutputTime,
} from '../../core/editor-helpers.js';
import { DEFAULT_VOLUME_PERCENT, previewVolume } from '../../core/sound.js';
import { state } from '../../core/state.js';
import { createCropOverlay } from './crop-overlay.js';
import { createDisplayPreview } from './display-preview.js';
import { choices, section, slider, toggle } from './frame.js';
import { createPreviewFramePump } from './preview-frames.js';

registerIcons({
  scissors: 'M8.5 8.5L20 20M8.5 15.5L20 4M9 6.5a2.5 2.5 0 11-5 0 2.5 2.5 0 015 0zM9 17.5a2.5 2.5 0 11-5 0 2.5 2.5 0 015 0z',
});

// Above this size the audio track is not decoded for the waveform.
const MAX_WAVEFORM_BYTES = 80 * 1024 * 1024;
const PEAK_COUNT = 1600;
const STRIP_HEIGHT = 40;
const TRACK_HEIGHT = 96;
// Pointer distance (CSS pixels) grabbing a handle or starting a selection.
const GRAB = 7;
const DRAG_START = 4;

function loadMetadata(media) {
  return new Promise((resolve, reject) => {
    media.addEventListener('loadedmetadata', resolve, { once: true });
    media.addEventListener('error', () => reject(new Error('unreadable media')), { once: true });
  });
}

// Loudest sample of each slice of the audio track, from 0 to 1.
async function readPeaks(blob) {
  const Offline = window.OfflineAudioContext || window.webkitOfflineAudioContext;
  if (!Offline || !blob || blob.size > MAX_WAVEFORM_BYTES) {
    return null;
  }
  // A low sample rate keeps the decoded track small: only its shape matters.
  const context = new Offline(1, 2, 22050);
  const buffer = await context.decodeAudioData(await blob.arrayBuffer());
  const samples = buffer.getChannelData(0);
  const slice = Math.max(1, Math.floor(samples.length / PEAK_COUNT));
  const stride = Math.max(1, Math.floor(slice / 256));
  const peaks = new Float32Array(Math.ceil(samples.length / slice));
  let loudest = 0;
  for (let index = 0; index < peaks.length; index += 1) {
    let peak = 0;
    const end = Math.min(samples.length, (index + 1) * slice);
    for (let cursor = index * slice; cursor < end; cursor += stride) {
      peak = Math.max(peak, Math.abs(samples[cursor]));
    }
    peaks[index] = peak;
    loudest = Math.max(loudest, peak);
  }
  if (loudest > 0) {
    for (let index = 0; index < peaks.length; index += 1) {
      peaks[index] /= loudest;
    }
  }
  return peaks;
}

// Builds the editor inside `frame`. `src` plays the media (an object URL of
// `blob` when its bytes could be read). `onChange()` fires after every
// committed action. `allowDisplayScale` adds the size the media takes on the
// overlay, with a live preview of it. Rejects when the media cannot be played.
export async function createAvEditor({ src, blob, kind, durationHint, artworkUrl, title, frame, onChange, allowDisplayScale = false, displayScalePercent = null, displayZone = null }) {
  const isVideo = kind === 'video';
  const media = h(isVideo ? 'video' : 'audio', { src, preload: 'auto', playsinline: true });
  await loadMetadata(media);
  const duration = Number.isFinite(media.duration) && media.duration > 0 ? media.duration : Number(durationHint) || 0;
  if (!(duration > 0)) {
    throw new Error('unknown duration');
  }

  let edit = defaultAvEdit(duration);
  let history = [JSON.stringify(edit)];
  let historyIndex = 0;
  let selection = null; // [start, end] in source seconds
  let cropMode = false;
  let cropRatio = 'free';
  let cropOverlay = null;
  let destroyed = false;
  let peaks = null;
  let strip = null;
  let graph = null; // { context, gain } once playback has started
  let lastPaintKey = '';

  // --- display scale (board item setting, no re-encode) ---
  let displayScale = normalizeDisplayScale(displayScalePercent);
  const initialScale = displayScale;
  let syncScaleFields = null;
  let syncPreviewToggle = null;
  let previewShown = false;
  let previewFrames = null;
  // The artwork of a sound is its picture on the overlay: it is what a scale
  // sizes.
  const artwork = !isVideo && artworkUrl ? h('img', { class: 'av-artwork', src: artworkUrl, alt: '' }) : null;

  // --- stage ---
  const inner = h('div', { class: 'av-inner' }, isVideo ? media : null);
  const videoFrame = h('div', { class: 'av-frame' }, inner);
  // Built only for a sound: appending `media` here would take a video out of
  // its frame.
  const audioStage = isVideo
    ? null
    : h(
        'div',
        { class: 'av-audio' },
        artwork || h('span', { class: 'av-glyph' }, icon('audio')),
        h('p', { class: 'ellipsis' }, title || ''),
        media,
      );
  const stageContent = isVideo ? videoFrame : audioStage;
  frame.stage.replaceChildren(stageContent);

  // --- display preview ---
  // The overlay sizes the picture of a video, and the artwork of a sound, the
  // same way: natural size × the scale chosen on the board, capped by the zone.
  function resultCrop() {
    return !cropMode && edit.crop ? edit.crop : { x: 0, y: 0, width: 1, height: 1 };
  }

  function resultTurns() {
    return cropMode ? 0 : edit.rotate / 90;
  }

  // Size of the picture once the recipe is applied: the crop, turned.
  function resultPictureSize() {
    if (!isVideo) {
      const width = artwork && artwork.naturalWidth > 0 ? artwork.naturalWidth : 0;
      const height = artwork && artwork.naturalHeight > 0 ? artwork.naturalHeight : 0;
      // A sound without artwork shows nothing to size; the card stands in for
      // it so the frame still reads.
      return width > 0 && height > 0 ? { width, height } : { width: 480, height: 480 };
    }
    const width = media.videoWidth || 0;
    const height = media.videoHeight || 0;
    if (!(width > 0) || !(height > 0)) {
      return null;
    }
    const crop = resultCrop();
    return rotatedSize(crop.width * width, crop.height * height, resultTurns());
  }

  function paintPreview(target) {
    const context = target.getContext('2d');
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, target.width, target.height);
    if (!isVideo) {
      if (artwork && artwork.naturalWidth > 0) {
        context.drawImage(artwork, 0, 0, target.width, target.height);
      }
      return;
    }
    if (!(media.videoWidth > 0) || !(media.videoHeight > 0)) {
      return;
    }
    const crop = resultCrop();
    const turns = ((resultTurns() % 4) + 4) % 4;
    const flipH = !cropMode && edit.flipH;
    const flipV = !cropMode && edit.flipV;
    const sourceWidth = crop.width * media.videoWidth;
    const sourceHeight = crop.height * media.videoHeight;
    // Buffer pixels per result pixel.
    const scale = target.width / rotatedSize(sourceWidth, sourceHeight, turns).width;
    context.save();
    context.translate(target.width / 2, target.height / 2);
    context.rotate(turns * (Math.PI / 2));
    context.scale(flipH ? -1 : 1, flipV ? -1 : 1);
    context.drawImage(
      media,
      crop.x * media.videoWidth,
      crop.y * media.videoHeight,
      sourceWidth,
      sourceHeight,
      -(sourceWidth * scale) / 2,
      -(sourceHeight * scale) / 2,
      sourceWidth * scale,
      sourceHeight * scale,
    );
    context.restore();
  }

  const preview = allowDisplayScale
    ? createDisplayPreview({
        stage: frame.stage,
        displayZone,
        frame: () => {
          const size = resultPictureSize();
          return size ? { ...size, scalePercent: displayScale } : null;
        },
        paint: paintPreview,
      })
    : null;
  if (preview) {
    frame.stage.append(preview.node);
    // The picture in the preview follows the playback, like a small overlay.
    const refreshPreview = () => {
      if (previewShown) {
        preview.requestPaint();
      }
    };
    media.addEventListener('timeupdate', refreshPreview);
    media.addEventListener('seeked', refreshPreview);
    // A video is painted for every frame it presents: `timeupdate` alone only
    // repaints it about four times a second, which plays like 4 fps.
    previewFrames = createPreviewFramePump({
      media,
      onFrame: () => preview.requestPaint(),
      isActive: () => previewShown && !destroyed,
    });
    media.addEventListener('play', () => previewFrames.start());
    media.addEventListener('pause', () => previewFrames.stop());
    if (artwork) {
      artwork.addEventListener('load', () => preview.invalidate(), { once: true });
    }
  }

  function layoutVideo() {
    if (!isVideo) {
      return;
    }
    const videoWidth = media.videoWidth || 16;
    const videoHeight = media.videoHeight || 9;
    const crop = !cropMode && edit.crop ? edit.crop : { x: 0, y: 0, width: 1, height: 1 };
    const turns = cropMode ? 0 : edit.rotate / 90;
    const flipH = !cropMode && edit.flipH;
    const flipV = !cropMode && edit.flipV;
    const cropWidth = crop.width * videoWidth;
    const cropHeight = crop.height * videoHeight;
    const shown = rotatedSize(cropWidth, cropHeight, turns);
    const scale = Math.min((frame.stage.clientWidth - 48) / shown.width, (frame.stage.clientHeight - 48) / shown.height);
    if (!(scale > 0)) {
      return;
    }
    videoFrame.style.width = `${shown.width * scale}px`;
    videoFrame.style.height = `${shown.height * scale}px`;
    inner.style.width = `${cropWidth * scale}px`;
    inner.style.height = `${cropHeight * scale}px`;
    // Rotation first, then the flips on the rotated picture: the order the
    // server applies them.
    inner.style.transform = `translate(-50%, -50%) scale(${flipH ? -1 : 1}, ${flipV ? -1 : 1}) rotate(${turns * 90}deg)`;
    media.style.width = `${videoWidth * scale}px`;
    media.style.height = `${videoHeight * scale}px`;
    media.style.left = `${-crop.x * videoWidth * scale}px`;
    media.style.top = `${-crop.y * videoHeight * scale}px`;
  }

  // --- timeline ---
  const canvas = h('canvas', { class: 'tl-canvas' });
  const track = h('div', { class: 'tl-track' }, canvas);
  const playButton = h('button', { class: 'btn tl-play', type: 'button', title: 'Lire / pause (Espace)', 'aria-label': 'Lire ou mettre en pause', onClick: () => togglePlay() }, icon('play'));
  const clock = h('span', { class: 'tl-clock' });
  const result = h('span', { class: 'tl-result' });
  frame.timeline.replaceChildren(
    h(
      'div',
      { class: 'tl-bar' },
      playButton,
      clock,
      h('span', { class: 'spacer' }),
      result,
    ),
    track,
  );
  frame.timeline.hidden = false;

  const timeToX = (time, width) => (time / duration) * width;
  const xToTime = (x, width) => clamp((x / width) * duration, 0, duration);

  function paintTimeline(force) {
    const width = track.clientWidth;
    if (!width) {
      return;
    }
    const key = [width, media.currentTime.toFixed(3), JSON.stringify(edit.segments), JSON.stringify(selection), peaks ? 1 : 0, strip ? strip.dataset.done : 0].join('|');
    if (!force && key === lastPaintKey) {
      return;
    }
    lastPaintKey = key;
    const ratio = Math.min(window.devicePixelRatio || 1, 2);
    if (canvas.width !== Math.round(width * ratio)) {
      canvas.width = Math.round(width * ratio);
      canvas.height = Math.round(TRACK_HEIGHT * ratio);
    }
    const context = canvas.getContext('2d');
    context.setTransform(ratio, 0, 0, ratio, 0, 0);
    context.clearRect(0, 0, width, TRACK_HEIGHT);
    context.fillStyle = '#171b25';
    context.fillRect(0, 0, width, TRACK_HEIGHT);

    const waveTop = isVideo ? STRIP_HEIGHT : 0;
    const waveHeight = TRACK_HEIGHT - waveTop;
    if (isVideo && strip) {
      context.drawImage(strip, 0, 0, width, STRIP_HEIGHT);
    }
    const middle = waveTop + waveHeight / 2;
    context.fillStyle = '#7fa8f5';
    if (peaks) {
      for (let x = 0; x < width; x += 2) {
        const peak = peaks[Math.min(peaks.length - 1, Math.floor((x / width) * peaks.length))];
        const bar = Math.max(1, peak * (waveHeight - 10));
        context.fillRect(x, middle - bar / 2, 1.4, bar);
      }
    } else {
      context.fillRect(0, middle - 0.5, width, 1);
    }

    // What was removed is darkened and hatched.
    for (const [start, end] of removedRanges(edit.segments, duration)) {
      const left = timeToX(start, width);
      const right = timeToX(end, width);
      context.fillStyle = 'rgba(13, 16, 23, 0.78)';
      context.fillRect(left, 0, right - left, TRACK_HEIGHT);
      context.save();
      context.beginPath();
      context.rect(left, 0, right - left, TRACK_HEIGHT);
      context.clip();
      context.strokeStyle = 'rgba(236, 106, 112, 0.35)';
      context.lineWidth = 1;
      for (let x = left - TRACK_HEIGHT; x < right; x += 9) {
        context.beginPath();
        context.moveTo(x, TRACK_HEIGHT);
        context.lineTo(x + TRACK_HEIGHT, 0);
        context.stroke();
      }
      context.restore();
    }

    // Edges of what is kept: the trim handles.
    for (const [start, end] of edit.segments) {
      for (const time of [start, end]) {
        const x = clamp(timeToX(time, width), 2, width - 2);
        context.fillStyle = '#e9ebf3';
        context.fillRect(x - 2, 0, 4, TRACK_HEIGHT);
        context.fillStyle = '#30384c';
        context.fillRect(x - 0.5, TRACK_HEIGHT / 2 - 9, 1, 18);
      }
    }

    if (selection) {
      const left = timeToX(selection[0], width);
      const right = timeToX(selection[1], width);
      context.fillStyle = 'rgba(242, 181, 68, 0.22)';
      context.fillRect(left, 0, right - left, TRACK_HEIGHT);
      context.fillStyle = '#f2b544';
      context.fillRect(left - 1, 0, 2, TRACK_HEIGHT);
      context.fillRect(right - 1, 0, 2, TRACK_HEIGHT);
    }

    const head = timeToX(media.currentTime, width);
    context.fillStyle = '#ffffff';
    context.fillRect(head - 1, 0, 2, TRACK_HEIGHT);
    context.beginPath();
    context.moveTo(head - 6, 0);
    context.lineTo(head + 6, 0);
    context.lineTo(head, 8);
    context.closePath();
    context.fill();
  }

  // Thumbnails along the timeline, read from a second, hidden video.
  async function buildStrip() {
    const probe = h('video', { src, muted: true, preload: 'auto', playsinline: true });
    await loadMetadata(probe);
    const width = Math.max(320, track.clientWidth);
    const tileHeight = STRIP_HEIGHT * 2;
    const tileWidth = Math.max(8, Math.round((tileHeight * (probe.videoWidth || 16)) / (probe.videoHeight || 9)));
    const count = clamp(Math.ceil((width * 2) / tileWidth), 4, 40);
    const tiles = document.createElement('canvas');
    tiles.width = count * tileWidth;
    tiles.height = tileHeight;
    tiles.dataset.done = '0';
    const context = tiles.getContext('2d');
    strip = tiles;
    for (let index = 0; index < count; index += 1) {
      if (destroyed) {
        break;
      }
      await new Promise((resolve) => {
        const timer = setTimeout(resolve, 2000);
        probe.addEventListener('seeked', () => { clearTimeout(timer); resolve(); }, { once: true });
        probe.currentTime = Math.min(duration - 0.05, ((index + 0.5) / count) * duration);
      });
      try {
        context.drawImage(probe, index * tileWidth, 0, tileWidth, tileHeight);
      } catch {
        break;
      }
      tiles.dataset.done = String(index + 1);
      paintTimeline();
    }
    probe.removeAttribute('src');
    probe.load();
  }

  // --- playback preview ---
  function applyGain() {
    const outTime = sourceToOutputTime(edit.segments, edit.speed, media.currentTime);
    const fades = fadeGain(outTime, outputDuration(edit.segments, edit.speed), edit.fadeIn, edit.fadeOut);
    const silent = edit.mute || edit.output === 'gif';
    const level = silent ? 0 : edit.volume * fades * previewVolume(state.activeGuildId);
    if (graph) {
      graph.gain.gain.value = level;
    } else {
      media.volume = clamp(level, 0, 1);
    }
  }

  // Volume above 100 % needs a gain node; it only works on bytes read by the
  // board itself (an object URL), never on a remote URL.
  function ensureGraph() {
    if (graph || !blob) {
      return;
    }
    const Context = window.AudioContext || window.webkitAudioContext;
    if (!Context) {
      return;
    }
    try {
      const context = new Context();
      const gain = context.createGain();
      context.createMediaElementSource(media).connect(gain);
      gain.connect(context.destination);
      media.volume = 1;
      graph = { context, gain };
    } catch {
      graph = null;
    }
  }

  function seek(time) {
    media.currentTime = clamp(time, 0, Math.max(0, duration - 0.01));
  }

  function setPlayIcon() {
    playButton.replaceChildren(icon(media.paused ? 'play' : 'pause'));
  }

  function togglePlay() {
    if (!media.paused) {
      media.pause();
      return;
    }
    ensureGraph();
    if (graph && graph.context.state === 'suspended') {
      graph.context.resume().catch(() => {});
    }
    const start = nextKeptTime(edit.segments, media.currentTime);
    if (start === null) {
      seek(edit.segments.length > 0 ? edit.segments[0][0] : 0);
    } else if (start > media.currentTime) {
      seek(start);
    }
    media.playbackRate = edit.speed;
    media.play().catch(() => {});
  }
  media.addEventListener('play', setPlayIcon);
  media.addEventListener('pause', setPlayIcon);

  function updateClock() {
    clock.textContent = `${formatClock(media.currentTime)} / ${formatClock(duration)}`;
  }

  function tick() {
    if (destroyed) {
      return;
    }
    if (!media.paused) {
      const next = nextKeptTime(edit.segments, media.currentTime);
      if (next === null) {
        media.pause();
        seek(edit.segments.length > 0 ? edit.segments[0][0] : 0);
      } else if (next > media.currentTime + 0.02) {
        seek(next);
      }
    }
    // Without `requestVideoFrameCallback` the animation loop redraws the
    // preview while it plays.
    if (isVideo && preview && previewShown && !media.paused && !previewFrames.supported) {
      preview.requestPaint();
    }
    applyGain();
    updateClock();
    paintTimeline();
    requestAnimationFrame(tick);
  }

  // --- history ---
  function afterChange() {
    media.playbackRate = edit.speed;
    const kept = outputDuration(edit.segments, edit.speed);
    result.textContent = isWhole(edit.segments, duration) && edit.speed === 1 ? `Durée\u00A0: ${formatClock(duration)}` : `Résultat\u00A0: ${formatClock(kept)} (au lieu de ${formatClock(duration)})`;
    layoutVideo();
    updateCutControls();
    paintTimeline(true);
    // A crop or a rotation changes the size the overlay will size, and the
    // chosen scale follows it.
    if (preview) {
      preview.invalidate();
    }
    frame.refreshHistory();
    if (onChange) {
      onChange();
    }
  }

  function commit() {
    edit.segments = normalizeSegments(edit.segments, duration);
    const saved = JSON.stringify(edit);
    if (saved !== history[historyIndex]) {
      history = history.slice(0, historyIndex + 1);
      history.push(saved);
      historyIndex = history.length - 1;
    }
    afterChange();
  }

  function restore(index) {
    historyIndex = index;
    edit = JSON.parse(history[index]);
    selection = null;
    setCropMode(false, true);
    renderPanel();
    afterChange();
  }

  // --- cutting ---
  const startField = h('input', { class: 'field tl-time', inputmode: 'decimal', 'aria-label': 'Début de la sélection', placeholder: '0:00,0', onChange: () => typedSelection() });
  const endField = h('input', { class: 'field tl-time', inputmode: 'decimal', 'aria-label': 'Fin de la sélection', placeholder: formatClock(duration), onChange: () => typedSelection() });
  const keepButton = h('button', { class: 'btn is-primary', type: 'button', title: 'Ne garder que la sélection (Entrée)', onClick: () => keepSelection() }, icon('crop'), 'Garder la sélection');
  const cutButton = h('button', { class: 'btn', type: 'button', title: 'Retirer la sélection (Suppr)', onClick: () => cutSelection() }, icon('scissors'), 'Supprimer la sélection');
  const resetCutButton = h('button', { class: 'btn is-quiet', type: 'button', onClick: () => { edit.segments = [[0, duration]]; selection = null; commit(); } }, 'Tout rétablir');

  function setSelection(next) {
    if (next) {
      const start = clamp(Math.min(next[0], next[1]), 0, duration);
      const end = clamp(Math.max(next[0], next[1]), 0, duration);
      selection = end - start >= 0.02 ? [start, end] : null;
    } else {
      selection = null;
    }
    updateCutControls();
    paintTimeline(true);
  }

  function updateCutControls() {
    if (document.activeElement !== startField) {
      startField.value = selection ? formatClock(selection[0]) : '';
    }
    if (document.activeElement !== endField) {
      endField.value = selection ? formatClock(selection[1]) : '';
    }
    keepButton.disabled = !selection;
    cutButton.disabled = !selection;
    resetCutButton.disabled = isWhole(edit.segments, duration);
  }

  function typedSelection() {
    const start = parseClock(startField.value);
    const end = parseClock(endField.value);
    const from = start === null ? (selection ? selection[0] : 0) : start;
    const to = end === null ? (selection ? selection[1] : duration) : end;
    setSelection([from, to]);
    // The fields show what was understood.
    startField.value = selection ? formatClock(selection[0]) : '';
    endField.value = selection ? formatClock(selection[1]) : '';
  }

  function keepSelection() {
    if (!selection) {
      return;
    }
    const kept = keepRange(edit.segments, selection[0], selection[1]);
    if (keptDuration(kept) < MIN_KEEP_SEC) {
      frame.setStatus('Cette sélection ne garde rien.');
      return;
    }
    edit.segments = kept;
    seek(selection[0]);
    selection = null;
    commit();
  }

  function cutSelection() {
    if (!selection) {
      return;
    }
    const kept = cutRange(edit.segments, selection[0], selection[1]);
    if (keptDuration(kept) < MIN_KEEP_SEC) {
      frame.setStatus('Il faut garder au moins un passage.');
      return;
    }
    edit.segments = kept;
    selection = null;
    commit();
  }

  // Marks the start or the end of the selection at the playhead.
  function markSelection(side) {
    const time = media.currentTime;
    if (side === 'start') {
      setSelection([time, selection && selection[1] > time ? selection[1] : duration]);
    } else {
      setSelection([selection && selection[0] < time ? selection[0] : 0, time]);
    }
  }

  // --- timeline pointer: trim handles, selection, seek ---
  function grabTarget(x, width) {
    if (selection) {
      if (Math.abs(x - timeToX(selection[0], width)) <= GRAB) {
        return { kind: 'selection', side: 0 };
      }
      if (Math.abs(x - timeToX(selection[1], width)) <= GRAB) {
        return { kind: 'selection', side: 1 };
      }
    }
    let best = null;
    edit.segments.forEach(([start, end], index) => {
      for (const [side, time] of [[0, start], [1, end]]) {
        const distance = Math.abs(x - clamp(timeToX(time, width), 2, width - 2));
        if (distance <= GRAB && (!best || distance < best.distance)) {
          best = { kind: 'edge', index, side, distance };
        }
      }
    });
    return best || { kind: 'free' };
  }

  track.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) {
      return;
    }
    event.preventDefault();
    frame.node.focus();
    const box = track.getBoundingClientRect();
    const startX = event.clientX - box.left;
    const target = grabTarget(startX, box.width);
    const startTime = xToTime(startX, box.width);
    let moved = false;
    track.setPointerCapture(event.pointerId);
    const move = (moveEvent) => {
      const x = clamp(moveEvent.clientX - box.left, 0, box.width);
      const time = xToTime(x, box.width);
      if (!moved && Math.abs(x - startX) < DRAG_START) {
        return;
      }
      moved = true;
      if (target.kind === 'edge') {
        const segments = edit.segments;
        const segment = segments[target.index];
        if (target.side === 0) {
          const floor = target.index > 0 ? segments[target.index - 1][1] : 0;
          segment[0] = clamp(time, floor, segment[1] - MIN_KEEP_SEC);
        } else {
          const ceiling = target.index < segments.length - 1 ? segments[target.index + 1][0] : duration;
          segment[1] = clamp(time, segment[0] + MIN_KEEP_SEC, ceiling);
        }
        seek(segment[target.side]);
      } else if (target.kind === 'selection') {
        const other = selection[1 - target.side];
        selection = target.side === 0 ? [Math.min(time, other - 0.02), other] : [other, Math.max(time, other + 0.02)];
        updateCutControls();
        seek(time);
      } else {
        selection = [Math.min(startTime, time), Math.max(startTime, time)];
        updateCutControls();
        seek(time);
      }
      paintTimeline(true);
    };
    const up = () => {
      track.removeEventListener('pointermove', move);
      track.removeEventListener('pointerup', up);
      track.removeEventListener('pointercancel', up);
      if (!moved) {
        // A plain click moves the playhead and drops the selection.
        seek(startTime);
        setSelection(null);
        return;
      }
      if (target.kind === 'edge') {
        commit();
      } else {
        setSelection(selection);
      }
    };
    track.addEventListener('pointermove', move);
    track.addEventListener('pointerup', up);
    track.addEventListener('pointercancel', up);
  });

  track.addEventListener('pointermove', (event) => {
    if (event.buttons) {
      return;
    }
    const box = track.getBoundingClientRect();
    track.style.cursor = grabTarget(event.clientX - box.left, box.width).kind === 'free' ? 'text' : 'ew-resize';
  });

  // --- video crop ---
  function cropAspect() {
    return (CROP_RATIOS.find((entry) => entry.id === cropRatio) || CROP_RATIOS[0]).ratio;
  }

  function storeCrop(rect) {
    const width = media.videoWidth;
    const height = media.videoHeight;
    const crop = { x: rect.x / width, y: rect.y / height, width: rect.width / width, height: rect.height / height };
    edit.crop = crop.width >= 0.999 && crop.height >= 0.999 ? null : crop;
  }

  function setCropMode(on, quiet) {
    cropMode = Boolean(on) && isVideo;
    if (cropOverlay) {
      cropOverlay.remove();
      cropOverlay = null;
    }
    layoutVideo();
    if (cropMode) {
      const width = media.videoWidth;
      const height = media.videoHeight;
      cropOverlay = createCropOverlay({
        host: inner,
        bounds: () => ({ width, height }),
        aspect: cropAspect,
        onChange: () => {},
        onCommit: (rect) => { storeCrop(rect); commit(); },
      });
      cropOverlay.set(
        edit.crop
          ? { x: edit.crop.x * width, y: edit.crop.y * height, width: edit.crop.width * width, height: edit.crop.height * height }
          : centeredCrop(cropAspect(), width, height),
      );
    }
    if (!quiet) {
      renderPanel();
    }
  }

  // With one mirror active, a rotation as seen on screen is the opposite
  // rotation of the picture before the mirror.
  function rotateView(direction) {
    const mirrored = edit.flipH !== edit.flipV;
    edit.rotate = (((edit.rotate + (mirrored ? -direction : direction) * 90) % 360) + 360) % 360;
    commit();
  }

  // --- panel ---
  function setSpeed(speed) {
    edit.speed = speed;
    commit();
    renderPanel();
  }

  // Any speed the server accepts, typed as "1,3" or "0.8".
  function speedField() {
    const custom = !SPEEDS.includes(edit.speed);
    const field = h('input', {
      class: `field speed-field${custom ? ' is-active' : ''}`,
      inputmode: 'decimal',
      'aria-label': `Vitesse personnalisée (de ${formatSpeed(MIN_SPEED)} à ${formatSpeed(MAX_SPEED)})`,
      title: `De ${formatSpeed(MIN_SPEED)} à ${formatSpeed(MAX_SPEED)}`,
      placeholder: '1,3',
      value: custom ? String(edit.speed).replace('.', ',') : '',
      onChange: () => {
        const speed = parseSpeed(field.value);
        if (speed === null) {
          field.value = custom ? String(edit.speed).replace('.', ',') : '';
          return;
        }
        setSpeed(speed);
      },
    });
    return h(
      'label',
      { class: 'editor-field' },
      h('span', null, 'Autre vitesse'),
      h('span', { class: 'editor-field-tail' }, field, h('span', { class: 'meta' }, '×')),
    );
  }

  // --- display scale (board item setting, no re-encode) ---
  function setPreviewShown(next) {
    if (!preview) {
      return;
    }
    previewShown = next;
    // The preview takes the stage over, like the picture editor does while its
    // display tool is open.
    stageContent.hidden = previewShown;
    if (previewShown) {
      preview.show();
      previewFrames.start();
    } else {
      previewFrames.stop();
      preview.hide();
    }
    if (syncPreviewToggle) {
      syncPreviewToggle();
    }
  }

  function setDisplayScale(next) {
    const value = normalizeDisplayScale(next);
    if (value === displayScale) {
      return;
    }
    displayScale = value;
    if (syncScaleFields) {
      syncScaleFields();
    }
    // The preview is the point of this setting: it follows every move.
    if (preview) {
      preview.invalidate();
    }
    afterChange();
  }

  // Size the media takes on the overlay: natural size × scale, capped by the
  // display zone minus its header.
  function displaySizeLine() {
    const size = resultPictureSize();
    if (!size || !preview) {
      return '';
    }
    const shown = displayedImageSize({
      width: size.width,
      height: size.height,
      scalePercent: displayScale,
      ...displayZoneLimits({ zoneWidth: preview.zone.width, zoneHeight: preview.zone.height }),
    });
    // Without a known zone the numbers come from the assumed 1080p screen, so
    // they are an estimate rather than a cap the overlay will apply.
    const note = preview.zone.known
      ? shown.capped
        ? ' (plafonné par la zone, en-tête compris)'
        : ''
      : ` (estimation sur une zone ${preview.zone.width}\u00A0×\u00A0${preview.zone.height}\u00A0px)`;
    return `À l’écran\u00A0: ${shown.width}\u00A0×\u00A0${shown.height}\u00A0px${note}`;
  }

  function sizePanel() {
    const picture = resultPictureSize() || { width: 0, height: 0 };
    const line = h('p', { class: 'meta' }, displaySizeLine());
    const value = h('span', { class: 'editor-value' }, `${displayScale}\u00A0%`);
    // Same span as the number field, so the presets never push the cursor out of range.
    const range = h('input', {
      type: 'range',
      min: String(MIN_DISPLAY_SCALE_PERCENT),
      max: String(MAX_DISPLAY_SCALE_PERCENT),
      step: '1',
      value: String(clamp(displayScale, MIN_DISPLAY_SCALE_PERCENT, MAX_DISPLAY_SCALE_PERCENT)),
      onInput: () => setDisplayScale(Number(range.value)),
    });
    resetOnDoubleClick(range, DEFAULT_DISPLAY_SCALE_PERCENT);
    const number = h('input', {
      class: 'field',
      type: 'number',
      min: String(MIN_DISPLAY_SCALE_PERCENT),
      max: String(MAX_DISPLAY_SCALE_PERCENT),
      step: '1',
      inputmode: 'numeric',
      value: String(displayScale),
      onInput: () => {
        const parsed = parseDisplayScale(number.value);
        if (parsed.ok) {
          setDisplayScale(parsed.value);
        }
      },
      onChange: () => {
        const parsed = parseDisplayScale(number.value);
        if (!parsed.ok) {
          number.value = String(displayScale);
        }
      },
    });
    // Both controls and the live size follow any change, whichever one moved.
    syncScaleFields = () => {
      value.textContent = `${displayScale}\u00A0%`;
      range.value = String(clamp(displayScale, MIN_DISPLAY_SCALE_PERCENT, MAX_DISPLAY_SCALE_PERCENT));
      number.value = String(displayScale);
      line.textContent = displaySizeLine();
    };
    const preset = (percent, label) =>
      h('button', { class: 'btn', type: 'button', onClick: () => setDisplayScale(percent) }, label);
    // Switching between the montage and the overlay preview swaps the stage, so
    // the button and its note are updated in place instead of rebuilding the
    // panel under the pointer.
    const toggle = h('button', { class: 'btn is-wide', type: 'button', onClick: () => setPreviewShown(!previewShown) });
    const note = h('p', { class: 'meta' });
    syncPreviewToggle = () => {
      toggle.textContent = previewShown ? 'Revenir au montage' : 'Aperçu sur l’overlay';
      toggle.classList.toggle('is-on', previewShown);
      toggle.setAttribute('aria-pressed', String(previewShown));
      note.hidden = !previewShown;
      note.textContent = `L’aperçu montre le média sur l’overlay, en-tête de ${OVERLAY_MEDIA_RESERVED_HEIGHT}\u00A0px compris. Une légende sous le média n’y est pas représentée.`;
    };
    syncPreviewToggle();
    return section(
      'Taille à l’affichage',
      h('p', { class: 'meta' }, 'Agrandit ou réduit le média sur l’overlay, sans le réencoder.'),
      h(
        'p',
        { class: 'meta' },
        `${isVideo ? 'Vidéo' : 'Pochette'}\u00A0: ${picture.width}\u00A0×\u00A0${picture.height}\u00A0px · `,
        preview && preview.zone.known
          ? `zone\u00A0: ${preview.zone.width}\u00A0×\u00A0${preview.zone.height}\u00A0px`
          : 'zone d’affichage inconnue',
      ),
      !isVideo && !(artwork && artwork.naturalWidth > 0)
        ? h('p', { class: 'meta' }, 'Ce son n’a pas de pochette\u00A0: l’échelle ne changera rien à l’écran.')
        : null,
      h('label', { class: 'editor-slider' }, h('span', { class: 'editor-slider-head' }, 'Échelle', value), range),
      h('label', { class: 'editor-field' }, h('span', null, 'Valeur\u00A0(%)'), number),
      h(
        'div',
        { class: 'editor-presets is-three' },
        preset(150, '150\u00A0%'),
        preset(200, '200\u00A0%'),
        preset(300, '300\u00A0%'),
      ),
      h(
        'div',
        { class: 'editor-presets is-two' },
        preset(MAX_DISPLAY_SCALE_PERCENT, 'Remplir la zone'),
        preset(DEFAULT_DISPLAY_SCALE_PERCENT, 'Taille d’origine'),
      ),
      line,
      toggle,
      note,
    );
  }

  function renderPanel() {
    const sound = edit.output !== 'gif';
    const picture = isVideo && edit.output !== 'audio';
    const sections = [
      section(
        'Découper',
        h('p', { class: 'meta' }, 'Tire les bords clairs de la frise pour raccourcir. Glisse sur la frise pour sélectionner un passage, puis garde-le ou supprime-le.'),
        h('div', { class: 'row' }, startField, h('span', { class: 'meta' }, '→'), endField),
        h(
          'div',
          { class: 'editor-pair' },
          h('button', { class: 'btn', type: 'button', title: 'La sélection commence à la tête de lecture (I)', onClick: () => markSelection('start') }, 'Début ici'),
          h('button', { class: 'btn', type: 'button', title: 'La sélection finit à la tête de lecture (O)', onClick: () => markSelection('end') }, 'Fin ici'),
        ),
        h('div', { class: 'editor-stack' }, keepButton, cutButton),
        resetCutButton,
      ),
      section(
        'Vitesse',
        choices(SPEEDS.map((speed) => [speed, formatSpeed(speed)]), edit.speed, setSpeed, 'Vitesse de lecture', 'is-grid-3'),
        speedField(),
        toggle('Lire à l’envers', edit.reverse, (reverse) => { edit.reverse = reverse; commit(); renderPanel(); }),
        edit.reverse ? h('p', { class: 'meta' }, 'L’aperçu reste à l’endroit ; le résultat sera inversé.') : null,
      ),
      sound
        ? section(
            'Son',
            isVideo ? toggle('Retirer le son', edit.mute, (mute) => { edit.mute = mute; commit(); renderPanel(); }) : null,
            edit.mute
              ? null
              : [
                  slider({ label: 'Volume', min: 0, max: 300, step: 5, value: Math.round(edit.volume * 100), defaultValue: DEFAULT_VOLUME_PERCENT, format: (value) => `${value}\u00A0%`, onInput: (value) => { edit.volume = value / 100; }, onCommit: commit }),
                  slider({ label: 'Fondu d’entrée', min: 0, max: 5, step: 0.1, value: edit.fadeIn, defaultValue: 0, format: (value) => `${value.toFixed(1).replace('.', ',')}\u00A0s`, onInput: (value) => { edit.fadeIn = value; }, onCommit: commit }),
                  slider({ label: 'Fondu de sortie', min: 0, max: 5, step: 0.1, value: edit.fadeOut, defaultValue: 0, format: (value) => `${value.toFixed(1).replace('.', ',')}\u00A0s`, onInput: (value) => { edit.fadeOut = value; }, onCommit: commit }),
                ],
          )
        : section('Son', h('p', { class: 'meta' }, 'Un GIF n’a pas de son.')),
      picture
        ? section(
            'Image',
            h(
              'div',
              { class: 'row is-wrap' },
              h('button', { class: `btn${cropMode ? ' is-primary' : ''}`, type: 'button', 'aria-pressed': String(cropMode), onClick: () => setCropMode(!cropMode) }, icon('crop'), cropMode ? 'Terminer le recadrage' : 'Recadrer'),
              edit.crop ? h('button', { class: 'btn is-quiet', type: 'button', onClick: () => { edit.crop = null; commit(); setCropMode(false); } }, 'Retirer le recadrage') : null,
            ),
            cropMode
              ? choices(CROP_RATIOS.map((entry) => [entry.id, entry.label]), cropRatio, (id) => {
                  cropRatio = id;
                  storeCrop(centeredCrop(cropAspect(), media.videoWidth, media.videoHeight));
                  commit();
                  setCropMode(true);
                }, 'Format du recadrage', 'is-grid-3')
              : null,
            h(
              'div',
              { class: 'row is-wrap' },
              h('button', { class: 'btn', type: 'button', title: 'Pivoter à gauche', 'aria-label': 'Pivoter à gauche', onClick: () => rotateView(-1) }, icon('rotateLeft')),
              h('button', { class: 'btn', type: 'button', title: 'Pivoter à droite', 'aria-label': 'Pivoter à droite', onClick: () => rotateView(1) }, icon('rotateRight')),
              h('button', { class: `btn${edit.flipH ? ' is-on' : ''}`, type: 'button', title: 'Miroir horizontal', 'aria-label': 'Miroir horizontal', 'aria-pressed': String(edit.flipH), onClick: () => { edit.flipH = !edit.flipH; commit(); renderPanel(); } }, icon('flipH')),
              h('button', { class: `btn${edit.flipV ? ' is-on' : ''}`, type: 'button', title: 'Miroir vertical', 'aria-label': 'Miroir vertical', 'aria-pressed': String(edit.flipV), onClick: () => { edit.flipV = !edit.flipV; commit(); renderPanel(); } }, icon('flipV')),
            ),
          )
        : null,
      // A sound without artwork has no picture to size; the card that stands in
      // for it ignores the scale, so the setting would do nothing.
      preview && showsDisplaySize({ isVideo, output: edit.output, hasArtwork: Boolean(artworkUrl) })
        ? sizePanel()
        : null,
      isVideo
        ? section(
            'Enregistrer en',
            choices([['same', 'Vidéo', 'video'], ['audio', 'Son', 'audio'], ['gif', 'GIF', 'image']], edit.output, (output) => {
              edit.output = output;
              if (output === 'audio') {
                setCropMode(false, true);
              }
              commit();
              renderPanel();
            }, 'Format du résultat', 'is-grid-3'),
          )
        : null,
    ];
    frame.panel.replaceChildren(...sections.filter(Boolean));
    frame.panel.hidden = false;
    updateCutControls();
  }

  const resizeObserver = new ResizeObserver(() => {
    layoutVideo();
    paintTimeline(true);
  });
  resizeObserver.observe(frame.stage);
  resizeObserver.observe(track);

  renderPanel();
  afterChange();
  setPlayIcon();
  requestAnimationFrame(tick);
  readPeaks(blob)
    .then((found) => {
      peaks = found;
      paintTimeline(true);
    })
    .catch(() => {});
  if (isVideo) {
    buildStrip().catch(() => {});
  }

  return {
    canUndo: () => historyIndex > 0,
    canRedo: () => historyIndex < history.length - 1,
    undo: () => {
      if (historyIndex > 0) {
        restore(historyIndex - 1);
      }
    },
    redo: () => {
      if (historyIndex < history.length - 1) {
        restore(historyIndex + 1);
      }
    },
    isDirty: () => history[historyIndex] !== history[0] || displayScale !== initialScale,
    pixelsDirty: () => history[historyIndex] !== history[0],
    scaleDirty: () => displayScale !== initialScale,
    displayScalePercent: () => displayScaleForStorage(displayScale),
    recipe: () => buildRecipe(edit, duration),
    problem: () => avEditProblem(edit, duration),
    output: () => edit.output,
    onKey(event, typing) {
      if (typing) {
        return false;
      }
      if (event.key === ' ') {
        // A focused button keeps its own Space.
        if (event.target instanceof HTMLButtonElement) {
          return false;
        }
        togglePlay();
        return true;
      }
      const key = event.key.toLowerCase();
      if (key === 'i') {
        markSelection('start');
        return true;
      }
      if (key === 'o') {
        markSelection('end');
        return true;
      }
      if ((event.key === 'Delete' || event.key === 'Backspace') && selection) {
        cutSelection();
        return true;
      }
      if (event.key === 'Enter' && selection && !(event.target instanceof HTMLButtonElement)) {
        keepSelection();
        return true;
      }
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        const step = (event.shiftKey ? 1 : 0.1) * (event.key === 'ArrowLeft' ? -1 : 1);
        seek(media.currentTime + step);
        return true;
      }
      if (event.key === 'Escape') {
        if (cropMode) {
          setCropMode(false);
          return true;
        }
        if (selection) {
          setSelection(null);
          return true;
        }
      }
      return false;
    },
    destroy() {
      destroyed = true;
      resizeObserver.disconnect();
      if (preview) {
        previewFrames.stop();
        preview.destroy();
      }
      media.pause();
      media.removeAttribute('src');
      media.load();
      if (graph) {
        graph.context.close().catch(() => {});
      }
    },
  };
}
