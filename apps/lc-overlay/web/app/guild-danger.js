// Destructive actions of a guild, shared by the guild settings page.
//
// Both actions need the guild's strong confirmation (typing its exact name) and
// call the overlay admin routes, which enforce the same `confirmGuildId` check
// as the server console. The purge closes the guild's sockets: the shell shows
// the removal notice and drops the guild, so nothing is refreshed here.

import { api, run } from './core/api.js';
import { h } from './core/dom.js';
import { confirmDialog, openDialog } from './core/ui.js';

// A native guild id IS the Discord server id; a standalone guild only reaches
// Discord once linked. Without a server, « Faire quitter le bot » has no target.
export function hasDiscordServer(guild) {
  return guild.standalone ? Boolean(guild.discordGuildId) : true;
}

// Strong confirmation: the destructive button stays disabled until the typed
// name matches exactly. `action` returns the request body (falsy on failure);
// `onSuccess` runs only after the server accepted it.
export function openTypedGuildDialog({ guildName, title, text, confirmLabel, inputId, action, onSuccess }) {
  openDialog((close) => {
    const field = h('input', {
      class: 'field',
      id: inputId,
      'aria-label': 'Nom de la guilde',
      autocomplete: 'off',
      placeholder: guildName,
    });
    const matches = () => field.value.trim() === guildName;
    const error = h('p', { class: 'error-text', role: 'alert', hidden: true }, 'Le nom ne correspond pas.');
    let busy = false;
    const confirmButton = h('button', { class: 'btn is-danger', type: 'button', disabled: true }, confirmLabel);
    const sync = () => {
      confirmButton.disabled = busy || !matches();
      if (matches()) {
        error.hidden = true;
      }
    };
    const submit = async () => {
      if (busy || !matches()) {
        error.hidden = false;
        field.focus();
        return;
      }
      busy = true;
      field.disabled = true;
      confirmButton.disabled = true;
      const body = await run(() => action(field.value));
      if (!body) {
        // run() already reported the failure: let the user try again.
        busy = false;
        field.disabled = false;
        sync();
        return;
      }
      close();
      if (onSuccess) {
        await onSuccess(body);
      }
    };
    confirmButton.addEventListener('click', submit);
    field.addEventListener('input', sync);
    field.addEventListener('keydown', (event) => {
      if (event.key !== 'Enter') {
        return;
      }
      event.preventDefault();
      if (matches()) {
        submit();
      } else {
        error.hidden = false;
      }
    });
    return [
      h('h2', null, title),
      h('p', null, text),
      h('label', { for: inputId }, `Saisis « ${guildName} » pour confirmer.`),
      field,
      error,
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', onClick: close }, 'Annuler'),
        confirmButton,
      ),
    ];
  });
}

// Deletes the guild: members, overlays, codes, queue, board and settings. The
// server closes the guild's overlay sockets (`4404 guild_deleted`), and the
// shell removes it from the guild list with its own notice.
export function openPurgeGuildDialog(guild) {
  openTypedGuildDialog({
    guildName: guild.name,
    title: `Supprimer « ${guild.name} » ?`,
    text: 'Membres, overlays, codes, file d’attente, board et réglages sont supprimés définitivement.',
    confirmLabel: 'Supprimer la guilde',
    inputId: 'purge-guild-name',
    action: () => api('POST', '/overlay/admin/purge', { confirmGuildId: guild.id }),
  });
}

// Makes the bot leave the guild's Discord server without deleting the guild.
export function confirmDiscordLeave(guild) {
  const name = guild.name || 'cette guilde';
  confirmDialog(
    'Faire quitter le bot ?',
    `Le bot quitte le serveur Discord de « ${name} ». Les commandes n’y fonctionneront plus ; les données restent sur le serveur LiveChat.`,
    'Faire quitter le bot',
    () =>
      run(
        () => api('POST', '/overlay/admin/discord-leave', { confirmGuildId: guild.id }),
        'Le bot a quitté le serveur Discord.',
      ),
  );
}
