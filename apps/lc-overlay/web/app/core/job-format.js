// Titre, origine et détails d'un job de la file, partagés par la file en
// attente, l'historique et la barre « à l'antenne ».
//
// Un job venu du board porte le titre de son item (`title`). Un média joué
// une fois ou ingéré par un client n'a pas d'item de board : le serveur
// expose alors `sourceTitle` et l'objet `media` (format, dimensions, taille,
// source) pour que la ligne reste lisible. Module pur (aucun DOM, aucun accès
// Tauri) pour rester testable sous `node --test`.

import { formatBytes, formatDuration } from './format.js';
import { KIND_LABELS } from './state.js';

// Payload riche encodé par `/ingest` quand un offset YouTube est demandé. Il
// n'est pas décodé (voir lc-playback) : il ne doit jamais s'afficher comme un
// titre ni comme une légende.
export const RICH_TEXT_PREFIX = '__LC_OVERLAY_RICH__::';

const UPLOAD_PREFIX = 'upload:';
const ORIGIN_LABELS = { board: 'Board', once: 'Une fois', direct: 'Direct' };
const ORIGIN_TITLES = {
  board: 'Média ajouté au board',
  once: 'Média joué une fois, hors board',
  direct: 'Texte envoyé directement à la file',
};

// Nom de fichier d'un média uploadé (`sourceUrl` = `upload:<nom>`), ou null.
export function uploadName(sourceUrl) {
  if (typeof sourceUrl !== 'string' || !sourceUrl.startsWith(UPLOAD_PREFIX)) {
    return null;
  }
  const name = sourceUrl.slice(UPLOAD_PREFIX.length);
  if (!name || name.startsWith('sha256:')) {
    return null;
  }
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

// Texte de job affichable, ou null : vide ou payload riche non décodé.
export function readableText(text) {
  const value = typeof text === 'string' ? text.trim() : '';
  if (!value || value.startsWith(RICH_TEXT_PREFIX)) {
    return null;
  }
  return value;
}

export function jobTitle(job) {
  if (!job) {
    return 'Média';
  }
  // Le titre d'un item de board prime sur `sourceTitle` (renommage possible),
  // puis viennent le nom du fichier uploadé et la légende.
  return (
    job.title ||
    job.sourceTitle ||
    uploadName(job.media && job.media.sourceUrl) ||
    readableText(job.text) ||
    (job.kind && KIND_LABELS[job.kind]) ||
    'Média'
  );
}

// « Board », « Une fois » ou « Direct », null si le serveur ne le dit pas.
export function jobOriginLabel(job) {
  return (job && ORIGIN_LABELS[job.origin]) || null;
}

export function jobOriginTitle(job) {
  return (job && ORIGIN_TITLES[job.origin]) || null;
}

// Hôte d'une source http(s), sans `www.` : null pour un upload ou une source
// illisible.
function sourceHost(sourceUrl) {
  if (typeof sourceUrl !== 'string') {
    return null;
  }
  try {
    const url = new URL(sourceUrl);
    if (url.protocol !== 'http:' && url.protocol !== 'https:') {
      return null;
    }
    return url.hostname.replace(/^www\./, '') || null;
  } catch {
    return null;
  }
}

// Seconde ligne d'un job : « Vidéo · 0:30 · 1280×720 · 2,1 Mo · youtu.be ».
// Null sans média (job direct).
export function jobMeta(job) {
  const media = job && job.media;
  if (!media) {
    return null;
  }
  const parts = [];
  const kind = KIND_LABELS[media.kind] || KIND_LABELS[job.kind];
  if (kind) {
    parts.push(kind);
  }
  const duration = formatDuration(media.durationSec || job.durationSec);
  if (duration) {
    parts.push(duration);
  }
  if (media.width > 0 && media.height > 0 && (media.kind === 'video' || media.kind === 'image')) {
    parts.push(`${media.width}×${media.height}`);
  }
  const size = formatBytes(media.sizeBytes);
  if (size) {
    parts.push(size);
  }
  const host = sourceHost(media.sourceUrl);
  if (host) {
    parts.push(host);
  }
  return parts.length ? parts.join(' · ') : null;
}
