import { useEffect, useMemo, useState } from 'preact/hooks';
import { getRights, isUnavailable, putRights } from '../api/client';
import type { RightsResponse } from '../api/types';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Empty,
  ErrorNotice,
  Loading,
  PageHead,
  Segmented,
  Table,
  Unavailable,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import {
  actionDetail,
  actionLabel,
  changedCellKeys,
  cloneRights,
  COLUMN_LABELS,
  enforcement,
  resetRightsToDefaults,
  rightsDirty,
  rightsPayload,
  setRightAllowed,
  SURFACE_INTROS,
  SURFACE_LABELS,
} from '../rights';
import { useAsync } from '../use-async';

export function RolesScreen() {
  const { route, notify } = useApp();
  const rights = useAsync((signal) => getRights(signal), []);
  const [saved, setSaved] = useState<RightsResponse | null>(null);
  const [draft, setDraft] = useState<RightsResponse | null>(null);
  const [surfaceId, setSurfaceId] = useState('overlay');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<unknown>(null);

  useEffect(() => {
    const data = rights.data;
    if (data) {
      setSaved(cloneRights(data));
      setDraft(cloneRights(data));
      setSurfaceId((current) =>
        data.surfaces.some((surface) => surface.id === current)
          ? current
          : (data.surfaces[0]?.id ?? 'overlay'),
      );
    }
  }, [rights.data]);

  const surfaces = draft?.surfaces ?? [];
  const surface = surfaces.find((item) => item.id === surfaceId) ?? surfaces[0];
  const dirty = saved && draft ? rightsDirty(saved, draft) : false;
  const changedKeys = useMemo(
    () => (saved && draft ? changedCellKeys(saved, draft) : new Set<string>()),
    [saved, draft],
  );

  const toggle = (actionId: string, role: string, checked: boolean) => {
    if (!draft || !surface) {
      return;
    }
    setDraft(setRightAllowed(draft, surface.id, actionId, role, checked));
  };

  const save = async () => {
    if (!saved || !draft || !dirty || saving) {
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const response = await putRights(rightsPayload(saved, draft));
      setSaved(cloneRights(response));
      setDraft(cloneRights(response));
      notify('Droits enregistrés et appliqués aux sessions ouvertes.');
    } catch (error) {
      setSaveError(error);
      notify(describeError(error), 'error');
    } finally {
      setSaving(false);
    }
  };

  const cancel = () => {
    if (saved) {
      setDraft(cloneRights(saved));
    }
  };

  const defaults = () => {
    if (draft) {
      setDraft(resetRightsToDefaults(draft));
    }
  };

  let content;
  if (rights.loading && !rights.data) {
    content = <Loading label="Chargement de la matrice des droits…" />;
  } else if (isUnavailable(rights.error)) {
    content = (
      <Unavailable detail="La matrice des droits (GET /admin/api/rights) n’est pas encore exposée par ce serveur." />
    );
  } else if (rights.error) {
    content = <ErrorNotice error={rights.error} onRetry={rights.reload} />;
  } else if (!surface || !draft || !saved) {
    content = <Empty>Aucune surface de droits n’est configurée.</Empty>;
  } else {
    const surfaceOptions = surfaces.map((item) => ({
      value: item.id,
      label: SURFACE_LABELS[item.id] ?? item.id,
    }));
    content = (
      <>
        <div class="toolbar">
          <Segmented
            value={surface.id}
            options={surfaceOptions}
            onChange={setSurfaceId}
            label="Surface"
          />
          <span class="spacer" />
          <p class="soft">Coche pleine : droit toujours accordé.</p>
        </div>

        {isUnavailable(saveError) ? (
          <Unavailable detail="L’enregistrement des droits (PUT /admin/api/rights) n’est pas exposé par ce serveur." />
        ) : saveError ? (
          <ErrorNotice error={saveError} title="Enregistrement impossible" />
        ) : null}

        <section class="card">
          <div class="card-head">
            <h2>{SURFACE_LABELS[surface.id] ?? surface.id}</h2>
          </div>
          <p class="card-intro soft">{SURFACE_INTROS[surface.id] ?? ''}</p>
          <Table className="rights">
            <thead>
              <tr>
                <th>Action</th>
                {surface.columns.map((column) => (
                  <th class="is-center" key={column}>
                    {COLUMN_LABELS[column] ?? column}
                  </th>
                ))}
                <th>Contrôle</th>
              </tr>
            </thead>
            <tbody>
              {surface.actions.map((action) => {
                const control = enforcement(surface.id, action.id);
                return (
                  <tr key={action.id}>
                    <td>
                      <span class="primary">{actionLabel(surface.id, action.id)}</span>
                      {actionDetail(surface.id, action.id) ? (
                        <span
                          class={`under${
                            actionDetail(surface.id, action.id).startsWith('/') ? ' mono' : ''
                          }`}
                        >
                          {actionDetail(surface.id, action.id)}
                        </span>
                      ) : null}
                    </td>
                    {surface.columns.map((column) => {
                      const cell = action.cells[column];
                      const label = `${actionLabel(surface.id, action.id)} — ${
                        COLUMN_LABELS[column] ?? column
                      }`;
                      if (!cell) {
                        return (
                          <td class="is-center" key={column}>
                            <span class="faint">—</span>
                          </td>
                        );
                      }
                      if (cell.fixed) {
                        return (
                          <td class="is-center" key={column}>
                            <span class="cell-fixed" title="Toujours accordé">
                              <Icon name="check" />
                              <span class="sr-only">{label} : toujours accordé</span>
                            </span>
                          </td>
                        );
                      }
                      const changed = changedKeys.has(`${surface.id}|${action.id}|${column}`);
                      return (
                        <td class="is-center" key={column}>
                          <label class={`cell-check${changed ? ' is-changed' : ''}`}>
                            <input
                              type="checkbox"
                              checked={cell.allowed}
                              onChange={(event) =>
                                toggle(action.id, column, event.currentTarget.checked)
                              }
                            />
                            <span class="sr-only">{label}</span>
                          </label>
                        </td>
                      );
                    })}
                    <td>
                      <Badge tone={control.className === 'is-ok' ? 'ok' : 'err'}>
                        {control.label}
                      </Badge>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </Table>

          {surface.id === 'discord' ? (
            <div class="card-body rights-extra">
              <label class="switch">
                <input
                  type="checkbox"
                  checked={draft.discordAdminIsAdmin}
                  onChange={(event) =>
                    setDraft({ ...draft, discordAdminIsAdmin: event.currentTarget.checked })
                  }
                />
                <span>
                  Un administrateur du serveur Discord a les droits « Admin » de ces commandes, même
                  sans compte lié
                </span>
              </label>
              <p class="hint">
                <span class="mono">/lier</span> n’est pas réglable : seul un administrateur du
                serveur Discord peut lier une guilde indépendante.
              </p>
            </div>
          ) : null}
        </section>

        <div class={`savebar${dirty ? ' is-dirty' : ''}`} role="status">
          <span class="soft">
            {dirty
              ? 'Modifications non enregistrées. Elles s’appliqueront à toutes les guildes et aux sessions ouvertes.'
              : 'Réglage de toute l’instance, identique pour toutes les guildes.'}
          </span>
          <span class="spacer" />
          <button class="btn" type="button" onClick={defaults}>
            Valeurs par défaut
          </button>
          <button class="btn" type="button" onClick={cancel} disabled={!dirty || saving}>
            Annuler
          </button>
          <button
            class="btn is-primary"
            type="button"
            onClick={() => void save()}
            disabled={!dirty || saving}
          >
            {saving ? 'Enregistrement…' : 'Enregistrer'}
          </button>
        </div>
      </>
    );
  }

  return (
    <Layout route={route} title="Rôles et droits" crumbs={[{ label: 'Rôles et droits' }]}>
      <PageHead
        title="Rôles et droits"
        sub="Ce que peuvent faire un invité, un membre et un admin de guilde, séparément dans l’overlay, l’extension et le bot Discord. La console elle-même reste réservée aux administrateurs du serveur."
      />
      {content}
    </Layout>
  );
}
