// Pure helpers of the board's selection mode and of the copy of media to
// another guild (no DOM, unit-tested in dev/tests/selection.test.mjs).

/** Most items the server copies in one request. */
export const IMPORT_BATCH_SIZE = 50;

/** Returns a new set with `id` added, or removed when it was there. */
export function toggleId(selected, id) {
  const next = new Set(selected);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  return next;
}

/**
 * Adds every id between `anchorId` and `id` (both included) in the visible
 * order. Without a visible anchor, only `id` is added.
 */
export function selectRange(selected, orderedIds, anchorId, id) {
  const next = new Set(selected);
  const from = anchorId === null ? -1 : orderedIds.indexOf(anchorId);
  const to = orderedIds.indexOf(id);
  if (from < 0 || to < 0) {
    next.add(id);
    return next;
  }
  const [start, end] = from <= to ? [from, to] : [to, from];
  for (const entry of orderedIds.slice(start, end + 1)) {
    next.add(entry);
  }
  return next;
}

/** Keeps only the selected ids still on the board. */
export function pruneSelection(selected, existingIds) {
  const existing = new Set(existingIds);
  return new Set([...selected].filter((id) => existing.has(id)));
}

function serverKey(url) {
  return String(url || '').trim().replace(/\/+$/, '').toLowerCase();
}

/**
 * Guilds the active guild's media can be copied to: the other guilds of the
 * same server where the device is not a guest (the server still decides).
 */
export function importTargets(guilds, activeGuildId) {
  const active = guilds.find((guild) => guild.membership.guildId === activeGuildId);
  if (!active) {
    return [];
  }
  const server = serverKey(active.membership.serverUrl);
  return guilds.filter(({ membership }) =>
    membership.guildId !== activeGuildId
    && serverKey(membership.serverUrl) === server
    && !membership.guestMode
    && membership.role !== 'guest');
}

/** Splits `ids` into requests of at most `size` ids. */
export function batches(ids, size = IMPORT_BATCH_SIZE) {
  const parts = [];
  for (let index = 0; index < ids.length; index += size) {
    parts.push(ids.slice(index, index + size));
  }
  return parts;
}

function count(value, singular, plural) {
  return `${value} ${value > 1 ? plural : singular}`;
}

/** Toast text describing the results of a copy to `guildName`. */
export function importSummary(results, guildName) {
  const tally = { added: 0, already: 0, not_found: 0, failed: 0 };
  for (const result of results) {
    if (result.status in tally) {
      tally[result.status] += 1;
    } else {
      tally.failed += 1;
    }
  }
  const parts = [];
  if (tally.already) {
    parts.push(count(tally.already, 'déjà présent', 'déjà présents'));
  }
  if (tally.not_found) {
    parts.push(count(tally.not_found, 'introuvable', 'introuvables'));
  }
  if (tally.failed) {
    parts.push(count(tally.failed, 'impossible', 'impossibles'));
  }
  if (tally.added === 0) {
    const tail = parts.length ? ` : ${parts.join(' · ')}` : '.';
    return {
      message: `Aucun média ajouté à ${guildName}${tail}`,
      isError: tally.not_found + tally.failed > 0,
    };
  }
  const head = `${count(tally.added, 'média ajouté', 'médias ajoutés')} à ${guildName}.`;
  return { message: [head, ...parts].join(' · '), isError: false };
}

/**
 * What Escape does on the board: close the detail drawer first, then leave
 * the selection mode. A key a context menu already handled (`handled`, its
 * `defaultPrevented`), an open dialog or a shortcut capture keeps it.
 */
export function escapeAction({ key, handled, capturing, dialogOpen, drawerOpen, selecting }) {
  if (key !== 'Escape' || handled || capturing || dialogOpen) {
    return null;
  }
  if (drawerOpen) {
    return 'closeDetail';
  }
  return selecting ? 'leaveSelection' : null;
}

/** What a click on a card's thumbnail does: `toggle`, `range` or `details`. */
export function cardClickAction({ selecting, shiftKey }) {
  if (!selecting) {
    return 'details';
  }
  return shiftKey ? 'range' : 'toggle';
}

/**
 * Media a card's context menu acts on: the whole selection (in board order)
 * when the card is picked, the card alone otherwise.
 */
export function menuTargetIds({ selecting, selection, itemId, orderedIds }) {
  if (selecting && selection.has(itemId)) {
    return orderedIds.filter((id) => selection.has(id));
  }
  return [itemId];
}
