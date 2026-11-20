// "Mon profil" dialog of a guild: display name, Discord account, extra
// device, browser extension pairing and leaving the guild.

import { api, run } from '../core/api.js';
import { resizeAvatar } from '../core/avatar.js';
import { avatarNode, h, icon } from '../core/dom.js';
import { formatCode } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { can, describeError, guildLabel, ROLE_LABELS, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { confirmDialog, copyText, openDialog } from '../core/ui.js';
import { applyGuilds, loadGuild } from '../data.js';

export function confirmLeaveGuild(guild) {
  confirmDialog(`Quitter ${guildLabel(guild)} ?`, 'Tu ne recevras plus ses médias. Il te faudra un nouveau code pour revenir.', 'Quitter la guilde', async () => {
    const result = await run(() => invoke('guild_leave', { payload: { guildId: guild.membership.guildId } }));
    if (!result) {
      return;
    }
    if (!result.ok) {
      notify(describeError(result.reason), true);
    }
    applyGuilds(result.guilds);
  });
}

export function openGuildDialog(guild) {
  openDialog((close) => {
    const member = (state.info && state.info.member) || {};
    const initialName = member.displayName || guild.membership.displayName || '';
    const name = h('input', { class: 'field', maxlength: '40', value: initialName });
    let avatar = member.avatar || null;
    let avatarChanged = false;
    const preview = h('span', { class: 'avatar-preview' }, avatarNode(initialName || '?', avatar));
    name.addEventListener('input', () => preview.replaceChildren(avatarNode(name.value || '?', avatar)));
    const removeAvatar = h(
      'button',
      { class: 'btn is-quiet profile-small', type: 'button', disabled: !avatar },
      'Retirer',
    );
    const file = h('input', {
      type: 'file',
      accept: 'image/png,image/jpeg,image/webp',
      hidden: true,
      onChange: async (event) => {
        const picked = event.target.files && event.target.files[0];
        if (!picked) {
          return;
        }
        const resized = await resizeAvatar(picked);
        if (!resized) {
          notify('Cette image ne peut pas être lue.', true);
          return;
        }
        avatar = resized;
        avatarChanged = true;
        preview.replaceChildren(avatarNode(name.value || '?', avatar));
        removeAvatar.disabled = false;
      },
    });
    removeAvatar.addEventListener('click', () => {
      avatar = null;
      avatarChanged = true;
      preview.replaceChildren(avatarNode(name.value || '?', avatar));
      removeAvatar.disabled = true;
    });
    const followsDiscord = member.profileSource === 'discord';
    const save = async () => {
      // Saving an unchanged profile would stop it from following Discord.
      if (name.value.trim() === initialName.trim() && !avatarChanged) {
        close();
        return;
      }
      const body = { displayName: name.value };
      // Unchanged avatar is omitted; an explicit null removes it.
      if (avatarChanged) {
        body.avatar = avatar;
      }
      const saved = await run(() => api('PATCH', '/overlay/profile', body), 'Profil enregistré.');
      if (saved) {
        close();
        loadGuild();
      }
    };
    const useDiscord = async () => {
      const body = await run(() => api('PATCH', '/overlay/profile', { useDiscord: true }));
      if (!body) {
        return;
      }
      notify(body.synced ? 'Profil Discord repris.' : 'Profil Discord repris : il sera lu à ta prochaine connexion.');
      close();
      loadGuild();
    };
    // Copies the text returned by `textOf`, read at click time: a code panel
    // exists before the code it will show.
    const copyButton = (label, message, textOf) =>
      h(
        'button',
        {
          class: 'btn is-quiet profile-copy',
          type: 'button',
          'aria-label': label,
          title: label,
          onClick: () => copyText(textOf(), message),
        },
        icon('copy'),
      );

    // --- another device ---
    let deviceText = '';
    const deviceValue = h('span', { class: 'profile-code' });
    const devicePanel = h(
      'div',
      { class: 'profile-code-panel', hidden: true },
      h(
        'div',
        { class: 'profile-code-main' },
        deviceValue,
        copyButton('Copier le code', 'Code de l’appareil copié.', () => deviceText),
        h('span', { class: 'profile-code-expiry' }, 'Expire dans 10 min'),
      ),
    );
    const makeDeviceCode = async () => {
      const body = await run(() => api('POST', '/overlay/device-code'));
      if (body && body.invite) {
        deviceText = formatCode(body.invite.code);
        deviceValue.textContent = deviceText;
        devicePanel.hidden = false;
      }
    };
    // --- browser extension ---
    let extensionUrlText = '';
    let extensionCodeText = '';
    const extensionUrl = h('span', { class: 'profile-value' });
    const extensionValue = h('span', { class: 'profile-code' });
    const extensionPanel = h(
      'div',
      { class: 'profile-code-panel', hidden: true },
      h(
        'div',
        { class: 'profile-field' },
        h('span', { class: 'profile-field-label' }, 'Adresse du serveur'),
        h(
          'div',
          { class: 'profile-field-value' },
          extensionUrl,
          copyButton('Copier l’adresse du serveur', 'Adresse du serveur copiée.', () => extensionUrlText),
        ),
      ),
      h(
        'div',
        { class: 'profile-field' },
        h('span', { class: 'profile-field-label' }, 'Code'),
        h(
          'div',
          { class: 'profile-field-value' },
          extensionValue,
          copyButton('Copier le code', 'Code de l’extension copié.', () => extensionCodeText),
        ),
      ),
      h('p', { class: 'profile-code-note' }, 'À saisir dans les options de l’extension ou dans le raccourci iOS. Expire dans 10 min.'),
    );
    const makeExtensionCode = async () => {
      const body = await run(() => api('POST', '/overlay/ingest-code'));
      if (!body || !body.code) {
        return;
      }
      extensionUrlText = body.apiBaseUrl;
      extensionCodeText = body.code;
      extensionUrl.textContent = body.apiBaseUrl;
      extensionValue.textContent = body.code;
      extensionPanel.hidden = false;
      copyText(body.code, 'Code de l’extension copié.');
    };
    // Linking needs a `/pair-code` code from the member's own Discord account:
    // the bot then accepts their commands.
    const profileSource = () => {
      if (!member.discordLinked) {
        const code = h('input', { class: 'field', maxlength: '12', placeholder: 'K7QX-M2PD', autocomplete: 'off', spellcheck: 'false' });
        const submit = async () => {
          if (!code.value.trim()) {
            code.focus();
            return;
          }
          const body = await run(() => api('POST', '/overlay/profile/discord-link', { code: code.value }));
          if (!body) {
            return;
          }
          notify(body.promoted ? 'Compte Discord lié : tu es maintenant admin de la guilde.' : 'Compte Discord lié.');
          close();
          loadGuild();
        };
        code.addEventListener('keydown', (event) => {
          if (event.key === 'Enter') {
            event.preventDefault();
            submit();
          }
        });
        const form = h(
          'div',
          { class: 'profile-link-form', hidden: true },
          h(
            'p',
            { class: 'file-hint' },
            'Sur le serveur Discord de la guilde, tape /pair-code puis saisis ici le code reçu : ton pseudo et ton avatar Discord remplaceront ceux affichés ici.',
          ),
          h(
            'div',
            { class: 'profile-link-row' },
            code,
            h('button', { class: 'btn', type: 'button', onClick: submit }, 'Lier'),
          ),
        );
        const open = h(
          'button',
          {
            class: 'btn is-quiet',
            type: 'button',
            onClick: () => {
              open.hidden = true;
              form.hidden = false;
              code.focus();
            },
          },
          'Lier mon compte Discord',
        );
        return h(
          'div',
          { class: 'profile-callout' },
          h('p', { class: 'profile-callout-text' }, 'Lie ton compte Discord pour utiliser les commandes du bot.'),
          open,
          form,
        );
      }
      if (followsDiscord) {
        return h(
          'div',
          { class: 'profile-callout' },
          h('p', { class: 'profile-callout-text' }, 'Compte Discord lié : ton pseudo et ton avatar en sont repris. Les modifier ici les fixe.'),
        );
      }
      return h(
        'div',
        { class: 'profile-callout' },
        h('p', { class: 'profile-callout-text' }, 'Pseudo et avatar choisis ici : ils ne suivent plus ton compte Discord.'),
        h('button', { class: 'btn is-quiet', type: 'button', onClick: useDiscord }, 'Reprendre mon profil Discord'),
      );
    };
    const leave = () => {
      close();
      confirmLeaveGuild(guild);
    };
    const identity = h(
      'section',
      { class: 'profile-section' },
      h('h3', { class: 'profile-section-title' }, 'Identité'),
      h(
        'div',
        { class: 'profile-identity' },
        preview,
        h(
          'div',
          { class: 'profile-identity-fields' },
          h('label', { class: 'profile-label' }, 'Pseudo', name),
          h(
            'div',
            { class: 'profile-identity-actions' },
            h('button', { class: 'btn is-quiet profile-small', type: 'button', onClick: () => file.click() }, 'Changer l’avatar'),
            removeAvatar,
            file,
          ),
        ),
      ),
    );
    const devices = h(
      'section',
      { class: 'profile-section' },
      h('h3', { class: 'profile-section-title' }, 'Appareils'),
      h(
        'div',
        { class: 'profile-devices' },
        h(
          'div',
          { class: 'profile-device' },
          h(
            'div',
            { class: 'profile-device-row' },
            h(
              'div',
              { class: 'profile-device-text' },
              h('p', { class: 'profile-device-title' }, 'Autre appareil'),
              h('p', { class: 'profile-device-desc' }, 'Rattache un autre PC à ton profil.'),
            ),
            h('button', { class: 'btn', type: 'button', onClick: makeDeviceCode }, 'Créer un code'),
          ),
          devicePanel,
        ),
        can('play')
          ? h(
              'div',
              { class: 'profile-device' },
              h(
                'div',
                { class: 'profile-device-row' },
                h(
                  'div',
                  { class: 'profile-device-text' },
                  h('p', { class: 'profile-device-title' }, 'Extension navigateur'),
                  h('p', { class: 'profile-device-desc' }, 'Envoie des médias depuis ton navigateur ou un Raccourci iOS.'),
                ),
                h('button', { class: 'btn', type: 'button', onClick: makeExtensionCode }, 'Créer un code'),
              ),
              extensionPanel,
            )
          : null,
      ),
    );
    return [
      h(
        'div',
        { class: 'profile-dialog' },
        h(
          'header',
          { class: 'profile-head' },
          h('h2', null, 'Mon profil'),
          h('p', { class: 'profile-subtitle' }, `${guildLabel(guild)} · ${(ROLE_LABELS[member.role] || 'membre').toLowerCase()}`),
        ),
        identity,
        profileSource(),
        devices,
        h(
          'div',
          { class: 'actions' },
          h('button', { class: 'btn is-quiet is-danger', onClick: leave }, 'Quitter la guilde'),
          h('span', { class: 'spacer' }),
          h('button', { class: 'btn is-quiet', onClick: close }, 'Fermer'),
          h('button', { class: 'btn', onClick: save }, 'Enregistrer'),
        ),
      ),
    ];
  });
}
