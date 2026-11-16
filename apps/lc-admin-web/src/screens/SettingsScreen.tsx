import { Fragment } from 'preact';
import { useEffect, useMemo, useState } from 'preact/hooks';
import { getRuntimeSettings, isUnavailable, patchRuntimeSettings } from '../api/client';
import type { RuntimeSettingsResponse } from '../api/types';
import { Layout } from '../components/Layout';
import { Icon } from '../components/icons';
import {
  Badge,
  Card,
  CardHead,
  ErrorNotice,
  Loading,
  PageHead,
  Switch,
  Unavailable,
} from '../components/ui';
import { useApp } from '../context';
import { describeError } from '../errors';
import { useAsync } from '../use-async';

type DraftValue = string | boolean | null;
type SettingKind = 'number' | 'text' | 'boolean';

interface SettingMeta {
  label: string;
  group: string;
  kind: SettingKind;
  hint?: string;
}

const GROUPS = [
  'Lecture',
  'Médias et quotas',
  'Overlays',
  'Synthèse vocale',
  'Autres réglages',
] as const;

const SETTING_META: Record<string, SettingMeta> = {
  defaultDuration: {
    label: 'Durée par défaut (s)',
    group: 'Lecture',
    kind: 'number',
    hint: 'Durée d’affichage d’une image ou d’un texte sans durée propre.',
  },
  mediaCacheTtlHours: { label: 'Cache : durée de vie (h)', group: 'Médias et quotas', kind: 'number' },
  mediaCacheMaxTotalMb: {
    label: 'Cache : taille maximale (Mo)',
    group: 'Médias et quotas',
    kind: 'number',
  },
  mediaBoardMaxTotalMb: {
    label: 'Board : quota par guilde (Mo)',
    group: 'Médias et quotas',
    kind: 'number',
  },
  mediaMaxSizeMb: {
    label: 'Taille maximale d’un média (Mo)',
    group: 'Médias et quotas',
    kind: 'number',
  },
  mediaVideoMaxHeight: {
    label: 'Hauteur vidéo maximale (p)',
    group: 'Médias et quotas',
    kind: 'number',
  },
  minOverlayVersion: {
    label: 'Version minimale d’un overlay',
    group: 'Overlays',
    kind: 'text',
    hint: 'Les overlays plus anciens sont refusés et invités à se mettre à jour.',
  },
  ttsEnabled: {
    label: 'Activer la synthèse vocale',
    group: 'Synthèse vocale',
    kind: 'boolean',
  },
};

const READ_ONLY_LABELS: Record<string, string> = {
  port: 'Port',
  API_URL: 'URL publique',
  apiUrl: 'URL publique',
  MEDIA_STORAGE_DIR: 'Dossier de stockage',
  mediaStorageDir: 'Dossier de stockage',
  DATABASE_URL: 'Base de données',
  databaseUrl: 'Base de données',
  DISCORD_TOKEN: 'Jeton du bot',
  discordToken: 'Jeton du bot',
  DISCORD_CLIENT_SECRET: 'Secret OAuth Discord',
  discordClientSecret: 'Secret OAuth Discord',
  ADMIN_UI_TOKEN: 'Secret d’administration',
  adminUiToken: 'Secret d’administration',
  PIPER_BIN: 'Binaire Piper',
  piperBin: 'Binaire Piper',
  FFMPEG_BINARY: 'Binaire ffmpeg',
  ffmpegBinary: 'Binaire ffmpeg',
  YTDLP_BINARY: 'Binaire yt-dlp',
  ytdlpBinary: 'Binaire yt-dlp',
};

const SOURCE_LABELS: Record<string, string> = {
  runtime: 'runtime (admin_runtime.json)',
  env: 'environnement',
  default: 'valeur par défaut',
};

const LEGACY_ORDER = [
  'defaultDuration',
  'mediaCacheTtlHours',
  'mediaCacheMaxTotalMb',
  'mediaBoardMaxTotalMb',
  'mediaMaxSizeMb',
  'mediaVideoMaxHeight',
  'minOverlayVersion',
  'ttsEnabled',
];

function metaFor(key: string, value: unknown): SettingMeta {
  const known = SETTING_META[key];
  if (known) {
    return known;
  }
  return {
    label: key,
    group: 'Autres réglages',
    kind: typeof value === 'boolean' ? 'boolean' : typeof value === 'number' ? 'number' : 'text',
  };
}

function toDraft(value: unknown): DraftValue {
  if (value == null) {
    return null;
  }
  if (typeof value === 'boolean') {
    return value;
  }
  if (typeof value === 'string') {
    return value;
  }
  if (typeof value === 'number') {
    return String(value);
  }
  return JSON.stringify(value);
}

function normalizeDraft(value: DraftValue, kind: SettingKind): unknown {
  if (value === null) {
    return null;
  }
  if (kind === 'boolean') {
    return Boolean(value);
  }
  if (kind === 'number') {
    if (typeof value === 'string' && value.trim() === '') {
      return null;
    }
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return String(value);
}

function displayReadOnly(value: unknown): string {
  if (typeof value === 'boolean') {
    return value ? 'Configuré' : 'Absent';
  }
  if (value == null) {
    return '—';
  }
  if (typeof value === 'object') {
    return JSON.stringify(value);
  }
  return String(value);
}

function scrollToSection(id: string) {
  document.getElementById(id)?.scrollIntoView({ block: 'start' });
}

export function SettingsScreen() {
  const { route, notify } = useApp();
  const settings = useAsync((signal) => getRuntimeSettings(signal), []);
  const [draft, setDraft] = useState<Record<string, DraftValue>>({});
  const [initial, setInitial] = useState<Record<string, DraftValue>>({});
  const [cookieText, setCookieText] = useState('');
  const [cookieInitial, setCookieInitial] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<unknown>(null);
  // The response the draft was built from: the form is shown only once the
  // draft matches the loaded settings, so an edit typed right after the load
  // is not overwritten by the initialization.
  const [draftSource, setDraftSource] = useState<unknown>(null);

  useEffect(() => {
    if (!settings.data) {
      return;
    }
    const next: Record<string, DraftValue> = {};
    for (const [key, entry] of Object.entries(settings.data.settings ?? {})) {
      next[key] = toDraft(entry.value);
    }
    setDraft(next);
    setInitial({ ...next });
    setCookieText(settings.data.tiktokCookie ?? '');
    setCookieInitial(
      typeof settings.data.tiktokCookie === 'string' ? settings.data.tiktokCookie : null,
    );
    setDraftSource(settings.data);
  }, [settings.data]);

  const fields = useMemo(() => {
    const entries = Object.entries(settings.data?.settings ?? {});
    return entries
      .map(([key, entry]) => ({ key, entry, meta: metaFor(key, entry.value) }))
      .sort((left, right) => {
        const leftOrder = LEGACY_ORDER.indexOf(left.key);
        const rightOrder = LEGACY_ORDER.indexOf(right.key);
        if (leftOrder !== -1 || rightOrder !== -1) {
          return (leftOrder === -1 ? 999 : leftOrder) - (rightOrder === -1 ? 999 : rightOrder);
        }
        return left.key.localeCompare(right.key);
      });
  }, [settings.data]);

  const grouped = useMemo(() => {
    const groups = new Map<string, typeof fields>();
    for (const field of fields) {
      const list = groups.get(field.meta.group) ?? [];
      list.push(field);
      groups.set(field.meta.group, list);
    }
    const order = (group: string): number => {
      const index = GROUPS.indexOf(group as (typeof GROUPS)[number]);
      return index === -1 ? GROUPS.length : index;
    };
    return [...groups.entries()].sort((left, right) => order(left[0]) - order(right[0]));
  }, [fields]);

  const readOnlyEntries = Object.entries(settings.data?.readOnly ?? {});
  const isLegacy = !settings.data?.settings;
  const showCookie = settings.data?.tiktokCookie !== undefined || settings.data?.hasTikTokCookie;
  const changedKeys = Object.keys(draft).filter((key) => draft[key] !== initial[key]);
  const cookieDirty = cookieInitial !== null && cookieText !== cookieInitial;
  const dirty = changedKeys.length > 0 || cookieDirty;

  const setValue = (key: string, value: DraftValue) => {
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const save = async () => {
    if (!dirty || saving) {
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const body: Record<string, unknown> = {};
      for (const key of changedKeys) {
        const field = fields.find((item) => item.key === key);
        body[key] = normalizeDraft(draft[key], field?.meta.kind ?? 'text');
      }
      if (cookieDirty) {
        body.tiktokCookie = cookieText;
      }
      await patchRuntimeSettings(body);
      notify('Paramètres du serveur enregistrés.');
      settings.reload();
    } catch (error) {
      setSaveError(error);
      notify(describeError(error), 'error');
    } finally {
      setSaving(false);
    }
  };

  let content;
  if (settings.loading && !settings.data) {
    content = <Loading label="Chargement des paramètres…" />;
  } else if (isUnavailable(settings.error)) {
    content = (
      <Unavailable detail="Les paramètres du serveur (GET /admin/api/runtime-settings) ne sont pas exposés par ce serveur." />
    );
  } else if (settings.error) {
    content = <ErrorNotice error={settings.error} onRetry={settings.reload} />;
  } else if (!settings.data || draftSource !== settings.data) {
    content = <Loading />;
  } else {
    const nav = [
      ...grouped.map(([group]) => ({ id: `set-${group.replace(/\s+/g, '-')}`, label: group })),
      ...(readOnlyEntries.length ? [{ id: 'set-environnement', label: 'Environnement' }] : []),
      ...(showCookie ? [{ id: 'set-sources', label: 'Cookie TikTok' }] : []),
    ];
    content = (
      <>
        {isLegacy ? (
          <div class="notice">
            <Icon name="info" />
            <div>
              <strong>Ce serveur expose le format historique des réglages.</strong>
              <div class="soft">
                Seul le cookie TikTok est modifiable à chaud ici. Les autres réglages restent
                définis par l’environnement du serveur.
              </div>
            </div>
          </div>
        ) : null}
        {isUnavailable(saveError) ? (
          <Unavailable detail="L’enregistrement des paramètres (PATCH /admin/api/runtime-settings) n’est pas exposé par ce serveur." />
        ) : saveError ? (
          <ErrorNotice error={saveError} title="Enregistrement impossible" />
        ) : null}

        <form
          class="settings-layout"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <nav class="settings-nav" aria-label="Sections des paramètres">
            {nav.map((item) => (
              <button
                type="button"
                key={item.id}
                onClick={() => scrollToSection(item.id)}
              >
                {item.label}
              </button>
            ))}
          </nav>

          <div class="stack">
            {grouped.map(([group, groupFields]) => (
              <Card key={group} id={`set-${group.replace(/\s+/g, '-')}`}>
                <CardHead title={group} />
                <div class="card-body">
                  <div class="form">
                    {groupFields.map(({ key, entry, meta }) => {
                      const value = draft[key];
                      const source = SOURCE_LABELS[entry.source] ?? entry.source;
                      return (
                        <Fragment key={key}>
                          <label
                            class="lbl"
                            for={meta.kind === 'boolean' ? undefined : `setting-${key}`}
                          >
                            {meta.label}
                            {meta.hint ? <p class="hint">{meta.hint}</p> : null}
                            <span class="env">{key}</span>
                          </label>
                          <div>
                            {meta.kind === 'boolean' ? (
                              <Switch
                                checked={value === true}
                                onChange={(checked) => setValue(key, checked)}
                                label={value === null ? 'Utiliser la valeur d’environnement' : 'Activé'}
                                hint={
                                  value === null
                                    ? 'Aucune valeur runtime : l’environnement décide.'
                                    : undefined
                                }
                              />
                            ) : (
                              <input
                                class="field"
                                id={`setting-${key}`}
                                type={meta.kind === 'number' ? 'number' : 'text'}
                                value={value == null ? '' : String(value)}
                                placeholder="Valeur d’environnement"
                                onInput={(event) => setValue(key, event.currentTarget.value)}
                              />
                            )}
                            <div class="row is-wrap">
                              <span class="hint">
                                Source : <Badge tone={entry.source === 'runtime' ? 'info' : undefined}>{source}</Badge>
                              </span>
                              <span class="spacer" />
                              <button
                                type="button"
                                class="btn is-small is-quiet"
                                onClick={() => setValue(key, null)}
                                disabled={value === null}
                              >
                                Valeur d’environnement
                              </button>
                            </div>
                          </div>
                        </Fragment>
                      );
                    })}
                  </div>
                </div>
              </Card>
            ))}

            {readOnlyEntries.length ? (
              <Card id="set-environnement">
                <CardHead title="Environnement (lecture seule)" meta="redémarrage requis" />
                <div class="card-body">
                  <dl class="kv">
                    {readOnlyEntries.map(([key, value]) => (
                      <Fragment key={key}>
                        <dt>
                          {READ_ONLY_LABELS[key] ?? READ_ONLY_LABELS[key.toUpperCase()] ?? key}
                          <span class="env">{key}</span>
                        </dt>
                        <dd class="mono">{displayReadOnly(value)}</dd>
                      </Fragment>
                    ))}
                  </dl>
                  <p class="hint">
                    Ces valeurs viennent de l’environnement du serveur. Les secrets ne sont jamais
                    renvoyés en clair : seul leur état est indiqué.
                  </p>
                </div>
              </Card>
            ) : null}

            {showCookie ? (
              <Card id="set-sources">
                <CardHead title="Source des médias" meta="cookie TikTok" />
                <div class="card-body">
                  <label for="tiktok-cookie">Cookie TikTok</label>
                  <textarea
                    class="field is-after-label mono"
                    id="tiktok-cookie"
                    rows={2}
                    placeholder="sessionid=…"
                    value={cookieText}
                    onInput={(event) => setCookieText(event.currentTarget.value)}
                  />
                  <p class="hint">
                    Nécessaire pour les TikTok restreints. Appliqué immédiatement après
                    enregistrement.
                  </p>
                  <p class="hint">
                    État :{' '}
                    <Badge tone={cookieText ? 'ok' : 'neutral'}>
                      {cookieText ? 'Configuré' : 'Absent'}
                    </Badge>
                  </p>
                </div>
              </Card>
            ) : null}

            <div class="settings-actions">
              <button class="btn is-primary" type="submit" disabled={!dirty || saving}>
                {saving ? 'Enregistrement…' : 'Enregistrer les paramètres'}
              </button>
              <span class="soft">
                {dirty
                  ? 'Modifications non enregistrées.'
                  : 'Les réglages marqués « environnement » demandent un redémarrage.'}
              </span>
              <span class="spacer" />
              <button
                type="button"
                class="btn"
                disabled={!dirty || saving}
                onClick={() => {
                  setDraft({ ...initial });
                  if (cookieInitial !== null) {
                    setCookieText(cookieInitial);
                  }
                }}
              >
                Annuler
              </button>
            </div>
          </div>
        </form>
      </>
    );
  }

  return (
    <Layout
      route={route}
      title="Paramètres du serveur"
      crumbs={[{ label: 'Paramètres du serveur' }]}
    >
      <PageHead
        title="Paramètres du serveur"
        sub="Réglages de toute l’instance. Les secrets, chemins et ports restent dans l’environnement et sont affichés en lecture seule."
      />
      {content}
    </Layout>
  );
}
