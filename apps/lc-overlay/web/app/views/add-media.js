// "Add media" dialog: a link, or one or more local files.
//
// Submitting closes the dialog immediately: the uploads run in the background
// and show as provisional cards on the board (see views/uploads.js). The two
// submit buttons each carry their action: "Ajouter au board" stores the media,
// "Jouer une fois" sends it to the overlays without adding it to the board.
// A member without `addMedia` but with `play` only gets "Jouer une fois". The
// Enter key always picks the primary action: adding when the member may add.
//
// "Jouer une fois" also takes a display time, for an image only: a video or an
// audio keeps its own length and the server ignores the value for it.

import { h } from '../core/dom.js';
import {
  INVALID_DURATION_MESSAGE,
  MAX_DISPLAY_DURATION_SEC,
  parseDisplayDuration,
} from '../core/format.js';
import { activeGuild, can, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { openDialog } from '../core/ui.js';
import { openFileImageEditor } from './editor/index.js';
import { startFileUpload, startLinkUpload } from './uploads.js';

function formatSize(bytes) {
  return `${((bytes || 0) / 1048576).toFixed(1).replace('.', ',')} Mo`;
}

// `options.url` prefills the link (a link dropped on the board).
export function openAddDialog(options) {
  const prefill = options && typeof options.url === 'string' ? options.url : '';
  openDialog((close) => {
    let mode = 'link';
    let chosenFiles = [];
    // The action the chosen submit button sets: adding to the board, or
    // playing once. A member who cannot add only gets "Jouer une fois", so it
    // is also the Enter key's action.
    const canAdd = can('addMedia');
    const canPlay = can('play');
    let playOnce = !canAdd && canPlay;
    // The text is a play caption: only a member who can play may type one,
    // and it is sent for "Jouer une fois" only, never stored on the board
    // item. It stays visible so nothing appears mid-dialog.
    const showMessage = canPlay;

    // --- link fields ---
    const linkUrl = h('input', { class: 'field', type: 'url', placeholder: 'https://…', required: true, value: prefill });
    const linkTitle = h('input', { class: 'field', placeholder: 'Titre du média', maxlength: '240' });
    const linkMessage = h('textarea', { class: 'field', rows: '2', maxlength: '500', placeholder: 'Légende affichée sur les overlays' });
    const linkMessageLabel = h(
      'label',
      { hidden: !showMessage },
      'Légende (facultatif)',
      linkMessage,
      h('span', { class: 'file-hint' }, 'Affichée sur les overlays pendant « Jouer une fois ».'),
    );
    // The display time is the length of a play-once image; a video or an audio
    // keeps its own length, so the field only shows when the media may be an
    // image. A chosen file reveals its kind in its MIME type, while a link's is
    // unknown until the server downloads it (the server decides then).
    const durationPlaceholder = (() => {
      const settings = state.info && state.info.guild && state.info.guild.settings;
      const seconds = settings && settings.defaultMediaTime;
      return Number.isFinite(seconds) && seconds > 0 ? `Par défaut (${seconds}\u00a0s)` : 'Par défaut';
    })();
    const mayBeImage = (file) => !file.type || file.type.startsWith('image/');
    const durationInput = () =>
      h('input', {
        class: 'field is-narrow',
        type: 'number',
        min: '1',
        max: String(MAX_DISPLAY_DURATION_SEC),
        step: '1',
        inputmode: 'numeric',
        placeholder: durationPlaceholder,
      });
    const durationLabel = (input) => h('label', null, 'Durée d’affichage (s)', input);
    const durationHint = () => h('p', { class: 'file-hint' }, 'Une image reste affichée ce temps\u00a0; une vidéo ou un son garde sa durée.');
    const linkDuration = durationInput();
    const linkDurationField = h('div', { class: 'stack', hidden: !canPlay }, durationLabel(linkDuration), durationHint());

    // --- file fields ---
    const fileInput = h('input', {
      type: 'file',
      multiple: true,
      accept: 'video/*,image/*,audio/*',
      hidden: true,
      onChange: () => {
        chosenFiles = Array.from(fileInput.files || []);
        renderFileList();
      },
    });
    const fileList = h('ul', { class: 'file-list', hidden: true });
    const fileTitle = h('input', { class: 'field', placeholder: 'Nom du fichier par défaut', maxlength: '240' });
    const fileTitleLabel = h('label', { hidden: true }, 'Titre (facultatif)', fileTitle);
    const multipleHint = h('p', { class: 'file-hint', hidden: true }, 'Chaque fichier utilisera son nom comme titre.');
    const fileMessage = h('textarea', { class: 'field', rows: '2', maxlength: '500', placeholder: 'Légende affichée sur les overlays' });
    const fileMessageLabel = h(
      'label',
      { hidden: !showMessage },
      'Légende (facultatif)',
      fileMessage,
      h('span', { class: 'file-hint' }, 'Affichée sur les overlays pendant « Jouer une fois ».'),
    );
    const fileDuration = durationInput();
    const fileDurationField = h('div', { class: 'stack', hidden: true }, durationLabel(fileDuration), durationHint());

    const linkPanel = h(
      'div',
      { class: 'stack' },
      h('label', null, 'Lien du média', linkUrl),
      h('label', null, 'Titre (facultatif)', linkTitle),
      linkMessageLabel,
      linkDurationField,
    );
    const filePanel = h(
      'div',
      { class: 'stack', hidden: true },
      h('div', { class: 'file-picker' }, h('button', { class: 'btn', type: 'button', onClick: () => fileInput.click() }, 'Choisir des fichiers'), fileInput),
      fileList,
      fileTitleLabel,
      multipleHint,
      fileMessageLabel,
      fileDurationField,
    );

    const linkButton = h('button', { class: 'btn mode-choice', type: 'button', 'aria-pressed': 'true', onClick: () => setMode('link') }, 'Lien');
    const fileButton = h('button', { class: 'btn mode-choice', type: 'button', 'aria-pressed': 'false', onClick: () => setMode('file') }, 'Fichier');
    // Each submit button carries its action. A member who cannot add only
    // gets "Jouer une fois"; a member who cannot play only adds. The DOM order
    // matches the visual order (secondary action, then primary); Enter is
    // handled below so it still picks the primary action.
    const submitAdd = canAdd
      ? h('button', { class: 'btn is-primary', type: 'submit', onClick: () => { playOnce = false; } }, 'Ajouter au board')
      : null;
    const submitOnce = canPlay
      ? h('button', { class: 'btn', type: 'submit', onClick: () => { playOnce = true; } }, 'Jouer une fois')
      : null;

    function updateSubmit() {
      const empty = mode === 'file' && chosenFiles.length === 0;
      if (submitAdd) {
        submitAdd.disabled = empty;
      }
      if (submitOnce) {
        submitOnce.disabled = empty;
      }
    }

    // A duration field that does not apply is hidden *and* disabled: an
    // invalid value typed before a mode switch (0, 700…) would otherwise still
    // block the form's native validation.
    function syncDuration() {
      const showLink = canPlay && mode === 'link';
      linkDuration.disabled = !showLink;
      linkDurationField.hidden = !canPlay;
      const showFile = canPlay && mode === 'file' && chosenFiles.some(mayBeImage);
      fileDuration.disabled = !showFile;
      fileDurationField.hidden = !showFile;
    }

    function setMode(next) {
      mode = next;
      const isLink = mode === 'link';
      linkButton.setAttribute('aria-pressed', String(isLink));
      fileButton.setAttribute('aria-pressed', String(!isLink));
      linkButton.classList.toggle('is-active', isLink);
      fileButton.classList.toggle('is-active', !isLink);
      linkPanel.hidden = !isLink;
      filePanel.hidden = isLink;
      linkUrl.disabled = !isLink;
      updateSubmit();
      syncDuration();
    }

    function renderFileList() {
      fileList.replaceChildren(
        ...chosenFiles.map((file, index) =>
          h(
            'li',
            null,
            h('span', { class: 'file-name ellipsis', title: file.name }, file.name),
            // A picture can be cropped or annotated before it is sent.
            file.type.startsWith('image/')
              ? h(
                  'button',
                  {
                    class: 'btn file-edit',
                    type: 'button',
                    onClick: () =>
                      openFileImageEditor(file, (edited) => {
                        chosenFiles[index] = edited;
                        renderFileList();
                      }),
                  },
                  'Modifier l’image',
                )
              : null,
            h('span', { class: 'file-size' }, formatSize(file.size)),
          ),
        ),
      );
      fileList.hidden = chosenFiles.length === 0;
      // One file: a single title can be given. Several: each keeps its name.
      fileTitleLabel.hidden = chosenFiles.length !== 1;
      multipleHint.hidden = chosenFiles.length <= 1;
      updateSubmit();
      syncDuration();
    }

    const form = h(
      'form',
      {
        class: 'stack',
        // Enter in a single-line field submits, like the form's default
        // behaviour, but always with the primary action. Buttons and the
        // textarea keep their native behaviour.
        onKeydown: (event) => {
          if (event.key !== 'Enter' || event.target.tagName !== 'INPUT') {
            return;
          }
          event.preventDefault();
          if (submitAdd) {
            playOnce = false;
            form.requestSubmit(submitAdd);
          } else if (submitOnce) {
            playOnce = true;
            form.requestSubmit(submitOnce);
          }
        },
        onSubmit: (event) => {
          event.preventDefault();
          const guildId = state.activeGuildId;
          if (!guildId) {
            close();
            return;
          }
          // A play-once is a playback request: a paused guild refuses it the
          // same way a board trigger does (playItem).
          if (playOnce) {
            const guild = activeGuild();
            if (guild && guild.membership.muted) {
              notify('Guilde en pause : reprends-la pour jouer.', true);
              return;
            }
          }
          // The display time only applies to a play-once image. A field hidden
          // for the current media is ignored, like its stale value.
          let displayDurationSec = null;
          if (playOnce) {
            const applies = mode === 'link' ? canPlay : !fileDurationField.hidden;
            if (applies) {
              const input = mode === 'link' ? linkDuration : fileDuration;
              const parsed = parseDisplayDuration(input.value);
              if (!parsed.ok) {
                notify(INVALID_DURATION_MESSAGE, true);
                input.focus();
                return;
              }
              displayDurationSec = parsed.value;
            }
          }
          if (mode === 'link') {
            const url = linkUrl.value.trim();
            if (!url) {
              linkUrl.reportValidity();
              return;
            }
            const payload = {
              guildId,
              url,
              title: linkTitle.value.trim(),
              message: playOnce ? linkMessage.value : '',
              playOnce,
              displayDurationSec,
            };
            close();
            startLinkUpload(payload);
            return;
          }
          if (chosenFiles.length === 0) {
            return;
          }
          const payloads = chosenFiles.map((file) => ({
            guildId,
            file,
            title: chosenFiles.length === 1 ? fileTitle.value.trim() : '',
            message: playOnce ? fileMessage.value : '',
            playOnce,
            // A video or an audio keeps its own length.
            displayDurationSec: mayBeImage(file) ? displayDurationSec : null,
          }));
          close();
          for (const payload of payloads) {
            startFileUpload(payload);
          }
        },
      },
      h('div', { class: 'add-media-modes', role: 'group', 'aria-label': 'Type d’ajout' }, linkButton, fileButton),
      linkPanel,
      filePanel,
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'),
        submitOnce,
        submitAdd,
      ),
    );

    setMode('link');
    // The link is the first thing to type; once prefilled, the title is next.
    setTimeout(() => (prefill ? linkTitle : linkUrl).focus(), 0);
    return [
      h('h2', null, 'Ajouter un média'),
      h('p', null, 'Ajoute une vidéo, une image ou un son depuis un lien ou un fichier.'),
      form,
    ];
  }, { width: 'wide' });
}
