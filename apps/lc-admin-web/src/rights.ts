/**
 * Pure model behind the « Rôles et droits » screen.
 *
 * The server owns the effective defaults and fixed cells (contract § 5); this
 * module only handles the editable copy, the dirty state and the payload sent
 * by PUT /admin/api/rights. Keeping it free of Preact makes it testable.
 */

import type {
  GuildRole,
  OverlaySessionMode,
  RightsAction,
  RightsResponse,
  RightsSurface,
  RightsUpdateCell,
  RightsUpdateRequest,
} from './api/types';

export const SURFACE_LABELS: Record<string, string> = {
  overlay: 'Overlay',
  extension: 'Extension',
  discord: 'Bot Discord',
};

export const SURFACE_INTROS: Record<string, string> = {
  overlay:
    'Le rôle du membre s’applique à tous ses appareils. Un appareil en session « Lecture seule » est traité comme un invité, quel que soit le rôle.',
  extension:
    'Une extension agit au nom de son auteur par défaut : c’est le rôle de ce membre qui compte. Aujourd’hui le rôle n’est vérifié qu’à l’appairage ; un membre rétrogradé ou exclu garde une extension qui fonctionne.',
  discord:
    'Le compte Discord doit être lié à un membre de la guilde (Profil → « Lier mon compte Discord » dans l’overlay). Un compte non lié n’a que les commandes ouvertes à tous.',
};

export const COLUMN_LABELS: Record<string, string> = {
  none: 'Compte non lié',
  guest: 'Invité',
  member: 'Membre',
  admin: 'Admin',
};

export const ROLE_LABELS: Record<GuildRole, string> = {
  admin: 'Admin',
  member: 'Membre',
  guest: 'Invité',
};

export const MODE_LABELS: Record<OverlaySessionMode, string> = {
  normal: 'Contrôle complet',
  invite_read_only: 'Lecture seule',
};

interface ActionMetadata {
  label: string;
  detail?: string;
}

export const ACTION_METADATA: Record<string, Record<string, ActionMetadata>> = {
  overlay: {
    receiveMedia: { label: 'Recevoir les médias' },
    viewQueue: { label: 'Voir la file d’attente' },
    viewBoard: { label: 'Consulter le board' },
    triggerPlayback: { label: 'Lancer une lecture' },
    stopPlayback: { label: 'Arrêter la lecture' },
    addMedia: { label: 'Ajouter des médias', detail: 'board, éditeur' },
    editOwnMedia: { label: 'Modifier ses médias', detail: 'titre, découpe, illustration' },
    editAnyMedia: { label: 'Modifier tous les médias' },
    moderateQueue: { label: 'Modérer la file', detail: 'retirer, vider' },
    manageMembers: { label: 'Gérer les membres', detail: 'rôle, exclusion' },
    manageInvites: { label: 'Gérer les invitations' },
    manageSettings: { label: 'Modifier les réglages de la guilde', detail: 'dont la liaison Discord' },
    purgeGuild: { label: 'Purger la guilde' },
  },
  extension: {
    pair: { label: 'Appairer une extension', detail: 'code créé depuis l’overlay' },
    send: { label: 'Envoyer un média à l’antenne', detail: 'POST /ingest' },
    addBoard: { label: 'Ranger le média dans le board', detail: 'option de l’extension' },
    author: { label: 'Signer au nom d’un autre membre' },
  },
  discord: {
    msg: { label: 'Afficher un message ou un média', detail: '/msg, /cmsg' },
    tts: { label: 'Faire parler la synthèse vocale', detail: '/dire, /cdire' },
    mediaAdd: { label: 'Ajouter un média au board', detail: '/media-add' },
    stop: { label: 'Arrêter la lecture', detail: '/stop' },
    overlays: { label: 'Lister les overlays connectés', detail: '/overlays' },
    pairCode: { label: 'Obtenir un code d’appairage', detail: '/pair-code' },
    info: { label: 'Aide et informations', detail: '/dispo, /help, /info' },
  },
};

export function actionLabel(surface: string, action: string): string {
  return ACTION_METADATA[surface]?.[action]?.label ?? action;
}

export function actionDetail(surface: string, action: string): string {
  return ACTION_METADATA[surface]?.[action]?.detail ?? '';
}

export interface Enforcement {
  label: string;
  className: string;
}

/** The « Contrôle » column of the mockup, derived from the documented state. */
export function enforcement(surface: string, action: string): Enforcement {
  if (surface === 'extension') {
    if (action === 'pair') {
      return { label: 'Vérifié par le serveur', className: 'is-ok' };
    }
    if (action === 'send') {
      return { label: 'À l’appairage seulement', className: 'is-err' };
    }
    return { label: 'Non vérifié aujourd’hui', className: 'is-err' };
  }
  return { label: 'Vérifié par le serveur', className: 'is-ok' };
}

export function cloneRights(rights: RightsResponse): RightsResponse {
  return {
    discordAdminIsAdmin: rights.discordAdminIsAdmin,
    surfaces: rights.surfaces.map((surface) => ({
      ...surface,
      columns: [...surface.columns],
      actions: surface.actions.map((action) => ({
        ...action,
        cells: Object.fromEntries(
          Object.entries(action.cells).map(([role, cell]) => [role, { ...cell }]),
        ),
      })),
    })),
  };
}

export function findSurface(rights: RightsResponse, surfaceId: string): RightsSurface | undefined {
  return rights.surfaces.find((surface) => surface.id === surfaceId);
}

export function findAction(
  rights: RightsResponse,
  surfaceId: string,
  actionId: string,
): RightsAction | undefined {
  return findSurface(rights, surfaceId)?.actions.find((action) => action.id === actionId);
}

/**
 * Update one editable cell, returning a new rights object. Fixed cells are
 * never altered (the server would reject the write with 400 fixed_right).
 */
export function setRightAllowed(
  rights: RightsResponse,
  surfaceId: string,
  actionId: string,
  role: string,
  allowed: boolean,
): RightsResponse {
  return {
    ...rights,
    surfaces: rights.surfaces.map((surface) => {
      if (surface.id !== surfaceId) {
        return surface;
      }
      return {
        ...surface,
        actions: surface.actions.map((action) => {
          if (action.id !== actionId) {
            return action;
          }
          const cell = action.cells[role];
          if (!cell || cell.fixed) {
            return action;
          }
          return { ...action, cells: { ...action.cells, [role]: { ...cell, allowed } } };
        }),
      };
    }),
  };
}

export function changedCells(saved: RightsResponse, draft: RightsResponse): RightsUpdateCell[] {
  const changes: RightsUpdateCell[] = [];
  for (const surface of draft.surfaces) {
    for (const action of surface.actions) {
      for (const [role, cell] of Object.entries(action.cells)) {
        const previous = findAction(saved, surface.id, action.id)?.cells[role];
        if (!previous || cell.fixed || previous.fixed) {
          continue;
        }
        if (previous.allowed !== cell.allowed) {
          changes.push({ surface: surface.id, action: action.id, role, allowed: cell.allowed });
        }
      }
    }
  }
  return changes;
}

export function rightsDirty(saved: RightsResponse, draft: RightsResponse): boolean {
  return changedCells(saved, draft).length > 0 || saved.discordAdminIsAdmin !== draft.discordAdminIsAdmin;
}

export function changedCellKeys(saved: RightsResponse, draft: RightsResponse): Set<string> {
  return new Set(changedCells(saved, draft).map((cell) => `${cell.surface}|${cell.action}|${cell.role}`));
}

/** Draft → payload for PUT /admin/api/rights. */
export function rightsPayload(saved: RightsResponse, draft: RightsResponse): RightsUpdateRequest {
  const cells = changedCells(saved, draft);
  const payload: RightsUpdateRequest = { cells };
  if (saved.discordAdminIsAdmin !== draft.discordAdminIsAdmin) {
    payload.discordAdminIsAdmin = draft.discordAdminIsAdmin;
  }
  return payload;
}

/** « Valeurs par défaut » button: every editable cell returns to `default`. */
export function resetRightsToDefaults(rights: RightsResponse): RightsResponse {
  return {
    ...rights,
    surfaces: rights.surfaces.map((surface) => ({
      ...surface,
      actions: surface.actions.map((action) => ({
        ...action,
        cells: Object.fromEntries(
          Object.entries(action.cells).map(([role, cell]) => [
            role,
            { ...cell, allowed: cell.fixed ? cell.allowed : cell.default },
          ]),
        ),
      })),
    })),
  };
}

export interface EffectivePermission {
  action: string;
  label: string;
  granted: boolean;
}

const PERMISSION_ORDER = [
  'receiveMedia',
  'viewQueue',
  'viewBoard',
  'triggerPlayback',
  'stopPlayback',
  'addMedia',
  'editOwnMedia',
  'editAnyMedia',
  'moderateQueue',
  'manageMembers',
  'manageInvites',
  'manageSettings',
  'purgeGuild',
];

/**
 * Effective overlay permissions for a device. A read-only session is treated
 * as a guest; the per-guild « invités voient le board » flag applies to the
 * board row just like on the server.
 */
export function effectivePermissions(
  rights: RightsResponse,
  role: GuildRole,
  sessionMode: OverlaySessionMode,
  guestsCanViewBoard: boolean,
): EffectivePermission[] {
  const effectiveRole: GuildRole = sessionMode === 'invite_read_only' ? 'guest' : role;
  return PERMISSION_ORDER.map((action) => {
    const cell = findAction(rights, 'overlay', action)?.cells[effectiveRole];
    let granted = Boolean(cell && (cell.fixed || cell.allowed));
    if (action === 'viewBoard' && effectiveRole === 'guest' && guestsCanViewBoard) {
      granted = true;
    }
    return { action, label: actionLabel('overlay', action), granted };
  });
}

export function roleLabel(role: GuildRole | null | undefined): string {
  return role ? ROLE_LABELS[role] ?? role : '—';
}
