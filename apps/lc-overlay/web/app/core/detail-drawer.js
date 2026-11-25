// Helpers describing a media of the board: the path of its signed URLs, the
// picture its card shows, where a card seeks a video, and the key of a rebuilt
// preview. The view state of the old side drawer used to live here; the detail
// window's own state is inside views/media-detail.js now.

// --- preview of the shown media ---

// Path of a signed media URL, without its query: the same asset keeps the
// same path while its signature changes every 15 minutes.
export function mediaPath(url) {
  if (typeof url !== 'string' || !url) {
    return null;
  }
  try {
    const parsed = new URL(url);
    return `${parsed.origin}${parsed.pathname}`;
  } catch {
    return url;
  }
}

// The picture of an audio item (embedded cover or chosen image), if any.
export function artworkOf(item) {
  const artwork = item && item.artwork;
  return artwork && typeof artwork.url === 'string' && artwork.url ? artwork : null;
}

// The picture of a video, once the server has made it: the platform's
// thumbnail of a link (YouTube, TikTok, X) or a frame of an uploaded video.
export function thumbnailOf(item) {
  const media = (item && item.media) || {};
  const thumbnail = media.kind === 'video' && item.thumbnail;
  return thumbnail && typeof thumbnail.url === 'string' && thumbnail.url ? thumbnail : null;
}

// The same signed URL with a version marker. A video's picture keeps its URL
// when it is replaced (the editor's frame rewrites the same cover), so the
// marker is what makes the browser fetch the new image instead of showing the
// one it already has.
export function withVersion(url, version) {
  if (typeof url !== 'string' || !url || version === null || version === undefined || version === '') {
    return url;
  }
  return `${url}${url.includes('?') ? '&' : '?'}v=${encodeURIComponent(version)}`;
}

// What a card compares to know whether its picture is still the same one: the
// path (a new signature is the same picture) and its version (a rewritten
// cover is a new picture).
export function pictureKey(item) {
  const picture = cardPicture(item);
  const thumbnail = (item && item.thumbnail) || null;
  const version = thumbnail && thumbnail.version !== null && thumbnail.version !== undefined ? thumbnail.version : '';
  return `${picture.kind}:${mediaPath(picture.url)}:${version}`;
}

// Where a card seeks its video while the server has no picture for it: a
// little way in, never the very first frame — many uploads open on a black
// fade-in, and `#t=0.1` showed a black card. `null` when the video's length is
// unknown (the browser then shows its first frame).
export function cardPreviewAt(item) {
  const media = (item && item.media) || {};
  if (media.kind !== 'video') {
    return null;
  }
  return previewAtWithin(Number(media.durationSec));
}

// The point a tenth of the way into a video, kept inside it: a tenth at most,
// and never its last frame. `null` when the length is unknown.
function previewAtWithin(duration) {
  if (!Number.isFinite(duration) || duration <= 0) {
    return null;
  }
  const at = Math.min(Math.max(duration * 0.1, 0.1), duration * 0.25, Math.max(duration - 0.1, 0.05));
  return Math.round(at * 1000) / 1000;
}

// Where a card seeks a video the browser turns out to be shorter than the
// server announced (an outdated length, a file re-encoded after the fact): the
// announced point would land past the end and paint a black card. `null` when
// the announced point already falls inside the video.
export function realignPreviewAt(previewAt, actualDuration) {
  if (!Number.isFinite(previewAt)) {
    return null;
  }
  const duration = Number(actualDuration);
  const inside = Number.isFinite(duration) && duration > 0 && previewAt <= duration - 0.05;
  return inside ? null : previewAtWithin(duration);
}

// What a board card shows: `{ url, kind }` with `kind` `image` or `video`
// (`url` null when there is nothing to load). An audio shows its picture, a
// video its thumbnail (a light image instead of loading the video), anything
// else the media itself.
export function cardPicture(item) {
  const media = (item && item.media) || {};
  const picture = media.kind === 'audio' ? artworkOf(item) : thumbnailOf(item);
  if (picture) {
    return { url: withVersion(picture.url, picture.version), kind: 'image' };
  }
  const mediaUrl = item && typeof item.mediaUrl === 'string' && item.mediaUrl ? item.mediaUrl : null;
  return { url: media.kind === 'audio' ? null : mediaUrl, kind: media.kind === 'video' ? 'video' : 'image' };
}

// What a built preview depends on: it is reused while this key is the same.
// The picture of an audio is part of it, so adding, replacing or removing the
// picture rebuilds the preview instead of leaving the old one in the drawer.
export function previewKey(item) {
  const media = (item && item.media) || {};
  const artwork = media.kind === 'audio' ? artworkOf(item) : null;
  return JSON.stringify([media.kind || null, mediaPath(item && item.mediaUrl), artwork ? mediaPath(artwork.url) : null]);
}
