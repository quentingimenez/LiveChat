// Shared toolbox of the media editors. The picture editor and the audio/video
// editor read the same ids, labels, glyphs and hints, so a tool reads the same
// in both and the two rails cannot drift apart.
//
// The panel of an editor shows the tool picked in its rail, and only that one.
// Three things stay outside the rule, on purpose: the selection inspector (it
// describes the object, not a tool), the timeline (it is the workspace of the
// audio/video editor) and the export format (it is a property of the result,
// set from the top bar).

import { showsDisplaySize } from '../../core/editor-helpers.js';

// `hints` is keyed by the editor's media; `default` covers the other ones.
const CATALOG = {
  select: {
    label: 'Sélection',
    glyph: 'cursor',
    hints: { default: 'Sélectionne un élément pour le déplacer, le redimensionner ou le modifier.' },
  },
  crop: {
    label: 'Recadrer',
    glyph: 'crop',
    hints: {
      image: 'Déplace les poignées, puis applique. Entrée valide.',
      video: 'Déplace les poignées sur la vidéo\u00A0: le résultat suit le cadre.',
    },
  },
  trim: {
    label: 'Découper',
    glyph: 'scissors',
    hints: {
      default: 'Tire les bords clairs de la frise pour raccourcir. Glisse sur la frise pour sélectionner un passage, puis garde-le ou supprime-le.',
    },
  },
  speed: {
    label: 'Vitesse',
    glyph: 'speed',
    hints: { default: 'Ralentis ou accélère tout le média.' },
  },
  sound: {
    label: 'Son',
    glyph: 'sound',
    hints: { default: 'Coupe le son, règle le volume et les fondus.' },
  },
  draw: {
    label: 'Dessiner',
    glyph: 'edit',
    hints: {
      image: 'Dessine directement sur l’image.',
      video: 'Dessine directement sur la vidéo.',
    },
  },
  shape: {
    label: 'Formes',
    glyph: 'shapes',
    hints: {
      image: 'Trace la forme en glissant sur l’image.',
      video: 'Trace la forme en glissant sur la vidéo.',
    },
  },
  text: {
    label: 'Texte',
    glyph: 'text',
    hints: {
      image: 'Clique sur l’image pour y poser un texte.',
      video: 'Clique sur la vidéo pour y poser un texte.',
    },
  },
  emoji: {
    label: 'Emoji',
    glyph: 'smile',
    hints: { default: 'Choisis un emoji\u00A0: il se pose au centre, déplace-le ensuite.' },
  },
  blur: {
    label: 'Masquer une zone',
    glyph: 'pixels',
    hints: { default: 'Trace un rectangle sur la zone à cacher.' },
  },
  adjust: {
    label: 'Lumière et couleurs',
    glyph: 'sliders',
    hints: {
      image: 'La lumière, les couleurs et le flou s’appliquent à toute l’image.',
      video: 'La lumière, les couleurs et le flou s’appliquent à toute la vidéo.',
    },
  },
  display: {
    label: 'Affichage',
    glyph: 'display',
    hints: {
      default: 'Choisis la taille sur l’overlay\u00A0: l’aperçu se met à jour pendant le réglage.',
    },
  },
};

// The rail of each editor, in reading order. `select` and `crop` share a group
// in the picture editor; the video editor adds the montage between that first
// group and the drawing, because the timeline sits under them.
const GROUPS = {
  image: [['select', 'crop'], ['draw', 'shape', 'text', 'emoji', 'adjust'], ['blur'], ['display']],
  video: [['select', 'crop'], ['trim', 'speed', 'sound'], ['draw', 'shape', 'text', 'emoji', 'adjust'], ['blur'], ['display']],
  audio: [['trim', 'speed', 'sound'], ['display']],
};

// Tools that work on the picture: none survives an audio-only video result.
const PICTURE_TOOLS = new Set(['select', 'crop', 'draw', 'shape', 'text', 'emoji', 'adjust', 'blur']);

export const EDITORS = Object.keys(GROUPS);

export function toolById(id) {
  return Object.prototype.hasOwnProperty.call(CATALOG, id) ? { id, ...CATALOG[id] } : null;
}

export function toolHint(id, media = 'image') {
  const entry = toolById(id);
  if (!entry) {
    return '';
  }
  return entry.hints[media] || entry.hints.default || '';
}

// The tools the rail shows for an editor, grouped. `allowDisplayScale` adds the
// size on the overlay, which only exists when the result shows a picture of
// its own: the frame of a video, or the artwork of a sound.
export function railGroups(editor, { allowDisplayScale = false, output = 'same', hasArtwork = false } = {}) {
  const groups = GROUPS[editor] || [];
  const isVideo = editor === 'video';
  const picture = editor === 'image' || (isVideo && output !== 'audio');
  const display = allowDisplayScale && (editor === 'image' || showsDisplaySize({ isVideo, output, hasArtwork }));
  return groups
    .map((group) => group.map(toolById).filter((entry) => entry && (entry.id !== 'display' || display) && (picture || !PICTURE_TOOLS.has(entry.id))))
    .filter((group) => group.length > 0);
}

// Which panel sections the active tool shows. `select` swaps its help for the
// inspector as soon as an object is selected; `display` keeps the cover next to
// the scale. Everything else is the tool's own section, and nothing else.
export function panelKeys(editor, tool, { hasSelection = false } = {}) {
  const known = (GROUPS[editor] || []).some((group) => group.includes(tool));
  if (!known) {
    throw new Error(`unknown ${editor} tool: ${tool}`);
  }
  if (tool === 'select') {
    return hasSelection ? ['selection'] : ['select'];
  }
  if (tool === 'display') {
    return ['display', 'cover'];
  }
  return [tool];
}
