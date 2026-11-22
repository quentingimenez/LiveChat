// Media editor entry points: edit a media of the board (image, audio or
// video), or retouch a local picture before it is sent.
//
// An edited picture is rendered here and sent like any file. An edited audio
// or video is described by a recipe the server applies. Either way the result
// becomes a new media of the board, or replaces the original.

import { editedFileName, editedTitle, exportMimeFor, recipeIsNoop } from '../../core/editor-helpers.js';
import { api } from '../../core/api.js';
import { can, state } from '../../core/state.js';
import { notify } from '../../core/toast.js';
import { requestItemsReload } from '../../data.js';
import { startEditJob, startFileUpload } from '../uploads.js';
import { currentDeviceSettings, guildLayout, loadDeviceSettings, selectedDisplay } from '../settings.js';
import { createAvEditor } from './av.js';
import { openEditorFrame } from './frame.js';
import { createImageEditor } from './image.js';

// The editor works on the media's bytes: a canvas or an audio decoder cannot
// read a remote URL directly.
async function fetchBlob(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return response.blob();
}

// A replaced media is overwritten for good: the bar asks before it happens.
const REPLACE_CONFIRM = {
  title: 'Remplacer l’original\u00A0?',
  text: 'Le média du board sera écrasé définitivement. Son titre, ses favoris et son raccourci sont conservés, mais l’ancien fichier ne pourra pas être récupéré.',
  confirmLabel: 'Remplacer définitivement',
  cancelLabel: 'Garder l’original',
};

// A media can be edited by whoever may add its result to the board, or
// replace it.
export function canOpenEditor(item) {
  return Boolean(item && item.mediaUrl && (can('addMedia') || item.canEdit));
}

// The title typed in the editor bar is the same board field the detail drawer
// edits: saved at once, and the board behind the editor is refreshed.
async function renameItem(item, next) {
  const value = String(next ?? '').trim();
  const body = await api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, { title: value });
  const saved = body && body.item && 'title' in body.item ? body.item.title : value || null;
  item.title = saved;
  requestItemsReload();
  return saved ?? '';
}

// Size of the active guild's display zone on this device, in physical pixels,
// when the shell knows it. `null` hides the capping hint in the editor.
async function displayZoneFor(guildId) {
  try {
    await loadDeviceSettings();
    const settings = currentDeviceSettings();
    if (!settings) {
      return null;
    }
    const displays = Array.isArray(settings.displays) ? settings.displays : [];
    const display = selectedDisplay(settings, displays);
    if (!display || !display.width || !display.height) {
      return null;
    }
    const layout = guildLayout(settings, guildId);
    const zone = { x: 0, y: 0, width: 1, height: 1, ...(layout.zone || {}) };
    return {
      width: Math.max(1, Math.round(zone.width * display.width)),
      height: Math.max(1, Math.round(zone.height * display.height)),
    };
  } catch {
    return null;
  }
}

async function editImage({ frame, item, title, blob, guildId }) {
  let editor = null;
  const mime = exportMimeFor(item.media.mime || blob.type);
  const displayZone = await displayZoneFor(guildId);
  if (frame.isClosed()) {
    return;
  }

  async function saveScale() {
    frame.setBusy('Enregistrement de la taille…');
    try {
      await api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, {
        displayScalePercent: editor.displayScalePercent(),
      });
    } catch (error) {
      frame.setBusy(null);
      notify(error.message, true);
      return;
    }
    frame.close();
    requestItemsReload();
    notify('Taille d’affichage enregistrée.');
  }

  // Board fields given to the saved picture. A replaced item keeps its own
  // fields, only a changed scale is written. A copy takes the scale chosen here
  // and the original's message, which an upload never stores itself.
  function copiedFields(replace) {
    const fields = {};
    if (!replace || editor.scaleDirty()) {
      fields.displayScalePercent = editor.displayScalePercent();
    }
    if (!replace && item.message) {
      fields.message = item.message;
    }
    return fields;
  }

  async function save(replace) {
    frame.setBusy('Préparation de l’image…');
    let result;
    try {
      result = await editor.exportBlob(mime);
    } catch {
      frame.setBusy(null);
      notify('L’image n’a pas pu être préparée.', true);
      return;
    }
    const file = new File([result], editedFileName(title, mime), { type: mime });
    frame.close();
    startFileUpload({
      guildId,
      file,
      title: replace ? title : editedTitle(title),
      replaceItemId: replace ? item.id : null,
      itemPatch: copiedFields(replace),
    });
  }

  function refresh() {
    const pixels = editor.pixelsDirty();
    const scale = editor.scaleDirty();
    // Only the display size changed: no file to upload, just the item field.
    if (!pixels && scale) {
      frame.setActions(
        item.canEdit
          ? [{
              label: 'Enregistrer la taille',
              title: 'Agrandit l’image sur l’overlay sans toucher au fichier',
              primary: true,
              onClick: saveScale,
            }]
          : [],
      );
      return;
    }
    frame.setActions([
      item.canEdit
        ? { label: 'Remplacer l’original…', title: 'Écrase le média du board ; son titre, ses favoris et son raccourci sont conservés', danger: true, disabled: !pixels, onClick: () => frame.confirm(REPLACE_CONFIRM, () => save(true)) }
        : null,
      can('addMedia') ? { label: 'Enregistrer une copie', title: 'Ajoute le résultat au board sans toucher à l’original', primary: true, disabled: !pixels, onClick: () => save(false) } : null,
    ].filter(Boolean));
  }

  try {
    editor = await createImageEditor({
      blob,
      frame,
      onChange: refresh,
      allowDisplayScale: true,
      displayScalePercent: item.displayScalePercent ?? null,
      displayZone,
      durationSec: item.media.durationSec ?? null,
    });
  } catch {
    frame.showError('Ce format d’image ne peut pas être ouvert dans l’éditeur.');
    return;
  }
  if (frame.isClosed()) {
    editor.destroy();
    return;
  }
  frame.setEditor(editor);
  refresh();
  if (/gif/i.test(item.media.mime || blob.type || '')) {
    notify('GIF animé\u00A0: l’éditeur ne garde que sa première image.');
  }
}

async function editAudioVideo({ frame, item, title, blob, guildId }) {
  const kind = item.media.kind;
  const objectUrl = blob ? URL.createObjectURL(blob) : null;
  const displayZone = await displayZoneFor(guildId);
  let editor = null;
  if (frame.isClosed()) {
    if (objectUrl) {
      URL.revokeObjectURL(objectUrl);
    }
    return;
  }

  async function saveScale() {
    frame.setBusy('Enregistrement de la taille…');
    try {
      await api('PATCH', `/overlay/media-board/items/${encodeURIComponent(item.id)}`, {
        displayScalePercent: editor.displayScalePercent(),
      });
    } catch (error) {
      frame.setBusy(null);
      notify(error.message, true);
      return;
    }
    frame.close();
    requestItemsReload();
    notify('Taille d’affichage enregistrée.');
  }

  // The picture of the video on the board: the frame the player shows. It
  // applies at once, outside the edit recipe, so the user keeps the editor
  // open (the frame is read straight from the stored video).
  async function saveCover(atSec) {
    frame.setBusy('Enregistrement de la miniature…');
    try {
      await api('POST', `/overlay/media-board/items/${encodeURIComponent(item.id)}/cover`, { atSec });
    } catch (error) {
      frame.setBusy(null);
      notify(error.message, true);
      return;
    }
    frame.setBusy(null);
    requestItemsReload();
    notify('Miniature enregistrée.');
  }

  // Board fields given to the saved result. A replaced item keeps its own
  // fields, only a changed scale is written. A copy takes the scale chosen here
  // and the original's message, which a job never stores itself.
  function copiedFields(replace) {
    const fields = {};
    if (!replace || editor.scaleDirty()) {
      fields.displayScalePercent = editor.displayScalePercent();
    }
    if (!replace && item.message) {
      fields.message = item.message;
    }
    return fields;
  }

  function save(replace) {
    const recipe = editor.recipe();
    const output = editor.output();
    const itemPatch = copiedFields(replace);
    frame.close();
    startEditJob({
      guildId,
      itemId: item.id,
      title: replace ? title : editedTitle(title),
      recipe,
      replace,
      mediaKind: output === 'audio' ? 'audio' : output === 'gif' ? 'image' : kind,
      itemPatch,
    });
  }

  function refresh() {
    // The editor reports its first state while it is still being built.
    if (!editor) {
      return;
    }
    const problem = editor.problem();
    const edited = !recipeIsNoop(editor.recipe());
    const blocked = Boolean(problem) || !edited;
    frame.setStatus(problem || '');
    // Only the display size changed: nothing to encode, just the item field.
    if (!edited && !problem && editor.scaleDirty()) {
      frame.setActions(
        item.canEdit
          ? [{
              label: 'Enregistrer la taille',
              title: 'Agrandit le média sur l’overlay sans le réencoder',
              primary: true,
              onClick: saveScale,
            }]
          : [],
      );
      return;
    }
    frame.setActions([
      item.canEdit && editor.output() === 'same'
        ? { label: 'Remplacer l’original…', title: 'Écrase le média du board ; son titre, ses favoris et son raccourci sont conservés', danger: true, disabled: blocked, onClick: () => frame.confirm(REPLACE_CONFIRM, () => save(true)) }
        : null,
      can('addMedia') ? { label: 'Enregistrer une copie', title: 'Ajoute le résultat au board sans toucher à l’original', primary: true, disabled: blocked, onClick: () => save(false) } : null,
    ].filter(Boolean));
  }

  try {
    editor = await createAvEditor({
      src: objectUrl || item.mediaUrl,
      blob,
      kind,
      durationHint: item.media.durationSec,
      artworkUrl: item.artwork && item.artwork.url,
      title,
      frame,
      onChange: refresh,
      allowDisplayScale: true,
      displayScalePercent: item.displayScalePercent ?? null,
      displayZone,
      // The picture belongs to the media, which only its editor may change.
      onCover: kind === 'video' && item.canEdit ? saveCover : null,
    });
  } catch {
    if (objectUrl) {
      URL.revokeObjectURL(objectUrl);
    }
    frame.showError(kind === 'video' ? 'Cette vidéo ne peut pas être lue dans l’éditeur.' : 'Ce son ne peut pas être lu dans l’éditeur.');
    return;
  }
  const destroy = editor.destroy;
  editor.destroy = () => {
    destroy();
    if (objectUrl) {
      URL.revokeObjectURL(objectUrl);
    }
  };
  if (frame.isClosed()) {
    editor.destroy();
    return;
  }
  frame.setEditor(editor);
  refresh();
}

// Opens the editor on a media of the board. `title` is the title shown for
// the item (its own, or the fallback the board derives from its source).
export async function openMediaEditor(item, title) {
  if (!canOpenEditor(item)) {
    return;
  }
  const guildId = state.activeGuildId;
  const frame = openEditorFrame({
    title,
    titleValue: item.title || '',
    rename: item.canEdit ? (next) => renameItem(item, next) : null,
  });
  let blob = null;
  try {
    blob = await fetchBlob(item.mediaUrl);
  } catch {
    blob = null;
  }
  if (frame.isClosed()) {
    return;
  }
  if (item.media.kind === 'image') {
    if (!blob) {
      frame.showError('L’image n’a pas pu être chargée depuis le serveur.');
      return;
    }
    await editImage({ frame, item, title, blob, guildId });
    return;
  }
  // Without the bytes, the media still plays from its URL: only the waveform
  // and volumes above 100 % are missing from the preview.
  await editAudioVideo({ frame, item, title, blob, guildId });
}

// Retouches a local picture before it is sent. `onDone(file)` receives the
// edited picture.
export async function openFileImageEditor(file, onDone) {
  const frame = openEditorFrame({ title: file.name });
  const mime = exportMimeFor(file.type);
  let editor = null;

  async function save() {
    frame.setBusy('Préparation de l’image…');
    try {
      const result = await editor.exportBlob(mime);
      const stem = file.name.replace(/\.[^.]+$/, '');
      frame.close();
      onDone(new File([result], editedFileName(stem, mime), { type: mime }));
    } catch {
      frame.setBusy(null);
      notify('L’image n’a pas pu être préparée.', true);
    }
  }

  const refresh = () => frame.setActions([{ label: 'Utiliser cette image', primary: true, disabled: !editor.isDirty(), onClick: save }]);
  try {
    editor = await createImageEditor({ blob: file, frame, onChange: refresh });
  } catch {
    frame.showError('Ce format d’image ne peut pas être ouvert dans l’éditeur.');
    return;
  }
  if (frame.isClosed()) {
    editor.destroy();
    return;
  }
  frame.setEditor(editor);
  refresh();
}
