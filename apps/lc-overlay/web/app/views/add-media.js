// "Add media" dialog: a link, or one or more local files.
//
// Submitting closes the dialog immediately: the uploads run in the background
// and show as provisional cards on the board (see views/uploads.js).

import { h } from '../core/dom.js';
import { state } from '../core/state.js';
import { openDialog } from '../core/ui.js';
import { startFileUpload, startLinkUpload } from './uploads.js';

function formatSize(bytes) {
  return `${((bytes || 0) / 1048576).toFixed(1).replace('.', ',')} Mo`;
}

export function openAddDialog() {
  openDialog((close) => {
    let mode = 'link';
    let chosenFiles = [];

    // --- link fields ---
    const linkUrl = h('input', { class: 'field', type: 'url', placeholder: 'https://…', required: true });
    const linkTitle = h('input', { class: 'field', placeholder: 'Chat qui tombe', maxlength: '240' });
    const linkMessage = h('textarea', { class: 'field', rows: '2', maxlength: '500', placeholder: 'Texte affiché avec le média' });

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
    const fileMessage = h('textarea', { class: 'field', rows: '2', maxlength: '500', placeholder: 'Texte affiché avec chaque média' });

    const linkPanel = h(
      'div',
      { class: 'stack' },
      h('label', null, 'Lien du média', linkUrl),
      h('label', null, 'Titre (facultatif)', linkTitle),
      h('label', null, 'Message (facultatif)', linkMessage),
    );
    const filePanel = h(
      'div',
      { class: 'stack', hidden: true },
      h('div', { class: 'file-picker' }, h('button', { class: 'btn', type: 'button', onClick: () => fileInput.click() }, 'Choisir des fichiers'), fileInput),
      fileList,
      fileTitleLabel,
      multipleHint,
      h('label', null, 'Message (facultatif)', fileMessage),
    );

    const linkButton = h('button', { class: 'btn mode-choice', type: 'button', 'aria-pressed': 'true', onClick: () => setMode('link') }, 'Lien');
    const fileButton = h('button', { class: 'btn mode-choice', type: 'button', 'aria-pressed': 'false', onClick: () => setMode('file') }, 'Fichier');
    const submit = h('button', { class: 'btn', type: 'submit' }, 'Ajouter au board');

    function updateSubmit() {
      submit.disabled = mode === 'file' && chosenFiles.length === 0;
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
    }

    function renderFileList() {
      fileList.replaceChildren(
        ...chosenFiles.map((file) =>
          h(
            'li',
            null,
            h('span', { class: 'file-name ellipsis', title: file.name }, file.name),
            h('span', { class: 'file-size' }, formatSize(file.size)),
          ),
        ),
      );
      fileList.hidden = chosenFiles.length === 0;
      // One file: a single title can be given. Several: each keeps its name.
      fileTitleLabel.hidden = chosenFiles.length !== 1;
      multipleHint.hidden = chosenFiles.length <= 1;
      updateSubmit();
    }

    const form = h(
      'form',
      {
        class: 'stack',
        onSubmit: (event) => {
          event.preventDefault();
          const guildId = state.activeGuildId;
          if (!guildId) {
            close();
            return;
          }
          if (mode === 'link') {
            const url = linkUrl.value.trim();
            if (!url) {
              linkUrl.reportValidity();
              return;
            }
            const payload = { guildId, url, title: linkTitle.value.trim(), message: linkMessage.value };
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
            message: fileMessage.value,
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
      h('div', { class: 'actions' }, h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'), submit),
    );

    setMode('link');
    return [
      h('h2', null, 'Ajouter un média'),
      h('p', null, 'Vidéo, image ou son : lien TikTok, X, YouTube ou direct, ou fichier de ton ordinateur.'),
      form,
    ];
  });
}
