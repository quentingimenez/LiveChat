// Copy board media to another guild of the same server (dialog + requests).
//
// The shell's `board_import` command sends both guild tokens to the server;
// the copy points to the same media file, so it is immediate.

import { h } from '../core/dom.js';
import { guildAcronym } from '../core/format.js';
import { invoke } from '../core/ipc.js';
import { batches, importSummary, importTargets } from '../core/selection.js';
import { describeError, guildLabel, state } from '../core/state.js';
import { notify } from '../core/toast.js';
import { openDialog } from '../core/ui.js';

function guildChoice(guild, chosen, onChoose) {
  const label = guildLabel(guild);
  const iconUrl = guild.membership.guildIconUrl;
  const fallback = () => h('span', { class: 'guild-choice-icon' }, guildAcronym(label) || '?');
  // A dead icon URL falls back to the acronym, like the rail.
  const face = iconUrl
    ? h('img', { class: 'guild-choice-icon', src: iconUrl, alt: '', referrerpolicy: 'no-referrer', onError: () => face.replaceWith(fallback()) })
    : fallback();
  return h(
    'button',
    {
      class: 'guild-choice',
      type: 'button',
      role: 'radio',
      'aria-checked': String(chosen),
      'data-guild-id': guild.membership.guildId,
      onClick: onChoose,
    },
    face,
    h('span', null, label),
  );
}

// Sends `itemIds` to `target` in batches; returns every result, or `null`
// once a request failed (the failure is already shown).
async function runImport(sourceGuildId, target, itemIds) {
  const results = [];
  for (const part of batches(itemIds)) {
    let response;
    try {
      response = await invoke('board_import', {
        payload: { sourceGuildId, targetGuildId: target.membership.guildId, itemIds: part },
      });
    } catch (error) {
      notify(`Copie impossible : ${error && error.message ? error.message : error}`, true);
      return null;
    }
    if (!response.ok) {
      const code = response.body && response.body.error;
      notify(describeError(code, 'Copie impossible.'), true);
      return null;
    }
    results.push(...((response.body && response.body.results) || []));
  }
  return results;
}

// Opens the guild picker for `itemIds` (media of the active guild). `onDone`
// runs after a copy that reached the server.
export function openImportDialog(itemIds, onDone) {
  const targets = importTargets(state.guilds, state.activeGuildId);
  if (itemIds.length === 0 || targets.length === 0) {
    return;
  }
  const sourceGuildId = state.activeGuildId;
  let chosen = targets.length === 1 ? targets[0].membership.guildId : null;
  openDialog((close) => {
    const submit = h('button', { class: 'btn is-primary', type: 'button', disabled: chosen === null }, 'Ajouter à la guilde');
    const list = h('div', { class: 'guild-choices', role: 'radiogroup', 'aria-label': 'Guilde' });
    const renderChoices = () => {
      list.replaceChildren(
        ...targets.map((guild) =>
          guildChoice(guild, guild.membership.guildId === chosen, () => {
            chosen = guild.membership.guildId;
            submit.disabled = false;
            renderChoices();
          })),
      );
    };
    renderChoices();
    submit.addEventListener('click', async () => {
      const target = targets.find((guild) => guild.membership.guildId === chosen);
      if (!target) {
        return;
      }
      submit.disabled = true;
      submit.textContent = 'Ajout en cours…';
      const results = await runImport(sourceGuildId, target, itemIds);
      close();
      if (results === null) {
        return;
      }
      const summary = importSummary(results, guildLabel(target));
      notify(summary.message, summary.isError);
      if (onDone) {
        onDone();
      }
    });
    const count = itemIds.length;
    const plural = count > 1;
    return [
      h('h2', null, plural ? `Ajouter ${count} médias à une autre guilde` : 'Ajouter ce média à une autre guilde'),
      h(
        'p',
        null,
        plural
          ? 'Les médias sont ajoutés au board de la guilde choisie, avec leur titre, leur légende et leur image.'
          : 'Le média est ajouté au board de la guilde choisie, avec son titre, sa légende et son image.',
      ),
      list,
      h(
        'div',
        { class: 'actions' },
        h('button', { class: 'btn is-quiet', type: 'button', onClick: close }, 'Annuler'),
        submit,
      ),
    ];
  });
}
