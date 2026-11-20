// "Mon profil" dialog of a guild: display name, Discord account, personal
// devices (overlays, browser extension, iOS) and leaving the guild.

import { api, run } from '../core/api.js';
import { resizeAvatar } from '../core/avatar.js';
import { avatarNode, h, icon } from '../core/dom.js';
import { formatCode } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import {
  canRevokePersonalDevice,
  personalDeviceCountLabel,
  personalDeviceKindLabel,
  personalDeviceSummary,
} from '../core/personal-devices.js';
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
        h('span', { class: 'profile-code-expiry' }, 'Expire dans 10\u00a0min'),
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
      h('p', { class: 'profile-code-note' }, 'À saisir dans les options de l’extension. Expire dans 10\u00a0min.'),
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
    // --- iOS shortcuts ---
    // A Shortcut cannot store a secret, so the board mints an ingest token and
    // asks for it once through the signed shortcut's import questions.
    let iosUrlText = '';
    let iosTokenText = '';
    const iosUrl = h('span', { class: 'profile-value' });
    const iosToken = h('span', { class: 'profile-value' });
    const shortcutLink = (label, file) =>
      h(
        'a',
        {
          class: 'btn is-quiet profile-small',
          href: `https://raw.githubusercontent.com/quentingimenez/LiveChat/main/ios/${file}`,
          target: '_blank',
          rel: 'noreferrer',
        },
        label,
      );
    const iosPanel = h(
      'div',
      { class: 'profile-code-panel', hidden: true },
      h(
        'div',
        { class: 'profile-field' },
        h('span', { class: 'profile-field-label' }, 'Adresse du serveur'),
        h(
          'div',
          { class: 'profile-field-value' },
          iosUrl,
          copyButton('Copier l’adresse du serveur', 'Adresse du serveur copiée.', () => iosUrlText),
        ),
      ),
      h(
        'div',
        { class: 'profile-field' },
        h('span', { class: 'profile-field-label' }, 'Jeton d’ingestion iOS'),
        h(
          'div',
          { class: 'profile-field-value' },
          iosToken,
          copyButton('Copier le jeton iOS', 'Jeton iOS copié.', () => iosTokenText),
        ),
      ),
      h(
        'p',
        { class: 'profile-code-note' },
        'Importe un raccourci ci-dessous, puis colle ces deux valeurs quand iOS les demande. Le jeton n’est affiché qu’une fois.',
      ),
      h(
        'div',
        { class: 'profile-shortcuts' },
        shortcutLink('Photo', 'LiveChat-Photo.shortcut'),
        shortcutLink('Vidéo', 'LiveChat-Video.shortcut'),
        shortcutLink('Lien', 'LiveChat-Lien.shortcut'),
      ),
    );
    const makeIosToken = async () => {
      const body = await run(() => api('POST', '/overlay/ingest-client'));
      if (!body || !body.ingestApiToken) {
        return;
      }
      iosUrlText = body.apiBaseUrl;
      iosTokenText = body.ingestApiToken;
      iosUrl.textContent = iosUrlText;
      iosToken.textContent = iosTokenText;
      iosPanel.hidden = false;
      copyText(iosTokenText, 'Jeton iOS copié.');
      loadPersonalDevices();
    };
    // --- personal devices ---
    // The same dialog administers the member's own devices: overlays, paired
    // extensions and iOS ingest clients. The server scopes the list to the
    // token's guild and member, and refuses to revoke the calling session.
    const personalList = h('div', { class: 'profile-personal-list' });
    const personalStatus = h('p', { class: 'profile-personal-empty' }, 'Chargement des appareils…');
    const personalCount = h('span', { class: 'profile-personal-count' });
    const personalDeviceRow = (device) => {
      const label = device.label || 'Appareil sans nom';
      const actions = h('div', { class: 'profile-personal-confirm' });
      let confirming = false;
      const renderActions = () => {
        actions.replaceChildren();
        if (!canRevokePersonalDevice(device)) {
          return;
        }
        if (!confirming) {
          actions.append(
            h(
              'button',
              {
                class: 'btn is-quiet is-danger profile-small',
                type: 'button',
                onClick: () => {
                  confirming = true;
                  renderActions();
                },
              },
              'Révoquer',
            ),
          );
          return;
        }
        actions.append(
          h('span', { class: 'profile-personal-confirm-text' }, 'Révoquer ?'),
          h(
            'button',
            {
              class: 'btn is-quiet profile-small',
              type: 'button',
              onClick: () => {
                confirming = false;
                renderActions();
              },
            },
            'Annuler',
          ),
          h(
            'button',
            {
              class: 'btn is-danger profile-small',
              type: 'button',
              onClick: async () => {
                const result = await run(
                  () =>
                    api(
                      'DELETE',
                      `/overlay/devices/${device.kind === 'overlay' ? 'overlay' : 'ingest'}/${encodeURIComponent(device.id)}`,
                    ),
                  'Appareil révoqué.',
                );
                if (result) {
                  loadPersonalDevices();
                  return;
                }
                confirming = false;
                renderActions();
              },
            },
            'Révoquer',
          ),
        );
      };
      renderActions();
      return h(
        'div',
        { class: 'profile-personal-row' },
        h(
          'div',
          { class: 'profile-personal-text' },
          h(
            'div',
            { class: 'profile-personal-name' },
            h('span', { class: 'badge' }, personalDeviceKindLabel(device.kind)),
            h('span', null, label),
            device.isSelf ? h('span', { class: 'badge is-off' }, 'Cet appareil') : null,
          ),
          h('p', { class: 'profile-personal-meta' }, personalDeviceSummary(device)),
        ),
        actions,
      );
    };
    const loadPersonalDevices = async () => {
      personalStatus.hidden = false;
      personalStatus.textContent = 'Chargement des appareils…';
      personalList.replaceChildren();
      const body = await run(() => api('GET', '/overlay/devices'));
      if (!body) {
        personalStatus.textContent = 'Impossible de charger tes appareils.';
        return;
      }
      const items = Array.isArray(body.items) ? body.items : [];
      personalCount.textContent = personalDeviceCountLabel(items.length);
      if (items.length === 0) {
        personalStatus.textContent = 'Aucun appareil rattaché à ton profil.';
        return;
      }
      personalStatus.hidden = true;
      personalList.replaceChildren(...items.map(personalDeviceRow));
    };
    const personalDevices = h(
      'div',
      { class: 'profile-personal' },
      h(
        'div',
        { class: 'profile-personal-head' },
        h(
          'div',
          { class: 'profile-personal-intro' },
          h('h4', { class: 'profile-personal-title' }, 'Mes appareils'),
          h('p', { class: 'profile-device-desc' }, 'Les appareils connectés à ton profil. Révoque ceux que tu ne reconnais pas.'),
        ),
        h(
          'div',
          { class: 'profile-personal-actions' },
          personalCount,
          h('button', { class: 'btn is-quiet profile-small', type: 'button', onClick: loadPersonalDevices }, 'Actualiser'),
        ),
      ),
      personalStatus,
      personalList,
    );
    loadPersonalDevices();
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
      personalDevices,
      h('h4', { class: 'profile-personal-add' }, 'Ajouter un appareil'),
      h('p', { class: 'profile-device-desc profile-add-intro' }, 'Associe un autre PC, ton navigateur ou un raccourci iOS.'),
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
                  h('p', { class: 'profile-device-desc' }, 'Envoie des médias depuis ton navigateur.'),
                ),
                h('button', { class: 'btn', type: 'button', onClick: makeExtensionCode }, 'Créer un code'),
              ),
              extensionPanel,
            )
          : null,
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
                  h('p', { class: 'profile-device-title' }, 'Raccourci iOS'),
                  h('p', { class: 'profile-device-desc' }, 'Partage une photo, une vidéo ou un lien depuis l’app Raccourcis.'),
                ),
                h('button', { class: 'btn', type: 'button', onClick: makeIosToken }, 'Configurer'),
              ),
              iosPanel,
            )
          : null,
      ),
    );
    return [
      h('h2', null, 'Mon profil'),
      h('p', { class: 'profile-subtitle' }, `${guildLabel(guild)} · ${(ROLE_LABELS[member.role] || 'membre').toLowerCase()}`),
      h(
        'div',
        { class: 'profile-dialog' },
        identity,
        profileSource(),
        devices,
        h(
          'div',
          { class: 'actions' },
          h('button', { class: 'btn is-quiet is-danger', onClick: leave }, 'Quitter la guilde'),
          h('span', { class: 'spacer' }),
          h('button', { class: 'btn is-quiet', onClick: close }, 'Annuler'),
          h('button', { class: 'btn is-primary', onClick: save }, 'Enregistrer'),
        ),
      ),
    ];
  }, { width: 'wide' });
}
