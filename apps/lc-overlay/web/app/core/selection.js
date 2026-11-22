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

/**
 * What a click on a card's thumbnail does: `pick` (Ctrl/Cmd+click outside the
 * selection mode), `toggle`, `range` or `details`.
 */
export function cardClickAction({ selecting, shiftKey, ctrlKey = false, metaKey = false }) {
  if (ctrlKey || metaKey) {
    return selecting ? 'toggle' : 'pick';
  }
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

// --- bulk actions of the selection bar ---

/**
 * The favorite action over the picked items: removing them all when every
 * one already is a favorite, adding only the missing ones otherwise.
 */
export function favoritePlan(items) {
  if (items.length > 0 && items.every((item) => item.favorite)) {
    return { label: 'Retirer des favoris', favorite: false, ids: items.map((item) => item.id) };
  }
  return {
    label: 'Ajouter aux favoris',
    favorite: true,
    ids: items.filter((item) => !item.favorite).map((item) => item.id),
  };
}

/** Picked items the member has the right to delete, in board order. */
export function deletableIds(items) {
  return items.filter((item) => item.canEdit).map((item) => item.id);
}

/**
 * The origin link of one item, http(s) only, or null: an uploaded file
 * (`upload:<name>`) has no page to copy or show.
 */
export function sourceLink(item) {
  const url = item && item.media && item.media.sourceUrl;
  return typeof url === 'string' && /^https?:\/\//i.test(url) ? url : null;
}

/**
 * Origin links of the picked items, one per media in board order, http(s)
 * only: an uploaded file has no page to paste.
 */
export function originLinks(items) {
  return items.map(sourceLink).filter(Boolean);
}

/** Toast text describing the outcome of playing the picked media in a row. */
export function playSummary(sent, failed) {
  const parts = [];
  if (failed > 0) {
    parts.push(count(failed, 'échec', 'échecs'));
  }
  if (sent === 0) {
    return { message: `Aucun média envoyé aux overlays${parts.length ? ` : ${parts.join(' · ')}` : '.'}`, isError: true };
  }
  return {
    message: [`${count(sent, 'média envoyé', 'médias envoyés')} aux overlays.`, ...parts].join(' · '),
    isError: failed > 0,
  };
}

/** Toast text describing the outcome of a bulk delete. */
export function deleteSummary(deleted, failed) {
  const parts = [];
  if (failed > 0) {
    parts.push(count(failed, 'échec', 'échecs'));
  }
  if (deleted === 0) {
    return { message: `Aucun média supprimé${parts.length ? ` : ${parts.join(' · ')}` : '.'}`, isError: true };
  }
  return {
    message: [`${count(deleted, 'média supprimé', 'médias supprimés')}.`, ...parts].join(' · '),
    isError: failed > 0,
  };
}

/** Toast text after the origin links of the picked media were copied. */
export function copiedLinksSummary(linkCount) {
  return `${count(linkCount, 'lien copié', 'liens copiés')}.`;
}

/**
 * Title and text of the bulk delete confirmation: how many media will go,
 * and how many picked ones cannot be deleted (the member lacks the right).
 */
export function deleteDialogText(deletableCount, forbiddenCount) {
  const title = deletableCount > 1 ? `Supprimer ${deletableCount} médias ?` : 'Supprimer ce média ?';
  const parts = [
    deletableCount > 1 ? 'Ils seront retirés du board de la guilde.' : 'Ce média sera retiré du board de la guilde.',
  ];
  if (forbiddenCount > 0) {
    parts.push(`${count(forbiddenCount, 'média ne peut pas être supprimé', 'médias ne peuvent pas être supprimés')} (droits).`);
  }
  return { title, text: parts.join(' ') };
}

/**
 * The selection bar's « Tout sélectionner » button is a toggle: when every
 * visible media is already picked it offers to clear the selection, otherwise
 * to pick them all. Hidden picks do not count, since the button only talks
 * about the cards on screen. An empty board never offers to clear.
 */
export function selectAllToggle({ orderedIds, selection }) {
  const ids = Array.isArray(orderedIds) ? orderedIds : [];
  const everyVisiblePicked = ids.length > 0 && ids.every((id) => selection.has(id));
  return everyVisiblePicked
    ? { label: 'Tout désélectionner', selectAll: false }
    : { label: 'Tout sélectionner', selectAll: true };
}

/**
 * Whether the key press picks every visible card: Ctrl+A (or Cmd+A) on the
 * board, outside as well as inside the selection mode. `canSelectAll` says
 * whether this window is the board with at least one visible card. A key a
 * context menu already handled, an open dialog, a shortcut capture or a field
 * being edited keeps its normal effect.
 */
export function selectAllAction({ key, ctrl, meta, handled, capturing, dialogOpen, editableTarget, canSelectAll }) {
  if (!canSelectAll || !(ctrl || meta) || handled || capturing || dialogOpen || editableTarget) {
    return null;
  }
  return key === 'a' || key === 'A' ? 'selectAll' : null;
}
