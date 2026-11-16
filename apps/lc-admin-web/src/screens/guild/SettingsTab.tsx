import { useEffect, useState } from 'preact/hooks';
import { isUnavailable, updateGuildSettings } from '../../api/client';
import type { UpdateGuildSettingsRequest } from '../../api/types';
import { Card, CardHead, ErrorNotice, Unavailable } from '../../components/ui';
import { describeError } from '../../errors';
import { numberOrNull } from './shared';
import type { GuildTabProps } from './types';

export function SettingsTab({ guild, detail, detailUnavailable, refresh, notify }: GuildTabProps) {
  // Initialised once from the guild; the parent remounts this tab (`key`)
  // when another guild is opened. Overview events replace the guild object
  // often, and resetting the form on each one would erase edits.
  const [name, setName] = useState(guild.editableName || guild.name);
  const [defaultTime, setDefaultTime] = useState(() =>
    guild.settings.overrides?.defaultMediaTime != null
      ? String(guild.settings.overrides.defaultMediaTime)
      : '',
  );
  const [maxTime, setMaxTime] = useState(() =>
    guild.settings.overrides?.maxMediaTime != null
      ? String(guild.settings.overrides.maxMediaTime)
      : '',
  );
  const [fullscreen, setFullscreen] = useState(guild.settings.displayMediaFull);
  const [guests, setGuests] = useState(detail?.guestsCanViewBoard ?? false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    setGuests(detail?.guestsCanViewBoard ?? false);
  }, [detail?.guestsCanViewBoard]);

  const save = async () => {
    if (busy) {
      return;
    }
    if (!name.trim()) {
      setError(new Error('Le nom de la guilde ne peut pas être vide.'));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const body: UpdateGuildSettingsRequest = {
        name: name.trim(),
        defaultMediaTime: numberOrNull(defaultTime),
        maxMediaTime: numberOrNull(maxTime),
        displayMediaFull: fullscreen,
      };
      if (detail) {
        body.guestsCanViewBoard = guests;
      }
      await updateGuildSettings(guild.id, body);
      notify('Réglages enregistrés et diffusés aux overlays connectés.');
      await refresh();
    } catch (caught) {
      setError(caught);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card>
      <CardHead title="Réglages de la guilde" meta="pour tous ses appareils" />
      <div class="card-body">
        <form
          class="form"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <label for="gs-name">Nom de la guilde</label>
          <div>
            <input
              class="field"
              id="gs-name"
              maxlength={60}
              value={name}
              placeholder={`Guild ${guild.id}`}
              onInput={(event) => setName(event.currentTarget.value)}
            />
          </div>

          <label for="gs-default">Durée par défaut (s)</label>
          <div>
            <input
              class="field"
              id="gs-default"
              type="number"
              min={0}
              max={86400}
              value={defaultTime}
              placeholder={`Valeur du serveur (${guild.settings.defaultMediaTime})`}
              onInput={(event) => setDefaultTime(event.currentTarget.value)}
            />
            <p class="hint">
              Durée d’affichage d’une image ou d’un texte sans durée propre. Vide = valeur du
              serveur.
            </p>
          </div>

          <label for="gs-max">Durée maximale (s)</label>
          <div>
            <input
              class="field"
              id="gs-max"
              type="number"
              min={0}
              max={86400}
              value={maxTime}
              placeholder="Sans limite"
              onInput={(event) => setMaxTime(event.currentTarget.value)}
            />
          </div>

          <span class="lbl">Affichage</span>
          <div>
            <label class="switch">
              <input
                type="checkbox"
                checked={fullscreen}
                onChange={(event) => setFullscreen(event.currentTarget.checked)}
              />
              <span>Médias en plein écran</span>
            </label>
          </div>

          <span class="lbl">Invités</span>
          <div>
            <label class="switch">
              <input
                type="checkbox"
                checked={guests}
                disabled={!detail}
                onChange={(event) => setGuests(event.currentTarget.checked)}
              />
              <span>Peuvent consulter le board</span>
            </label>
            {!detail ? (
              <p class="hint">
                {detailUnavailable
                  ? 'Indisponible sur ce serveur : le détail de la guilde n’est pas exposé.'
                  : 'Chargement du détail de la guilde…'}
              </p>
            ) : null}
          </div>

          <span />
          <div class="row">
            <button class="btn is-primary" type="submit" disabled={busy}>
              {busy ? 'Enregistrement…' : 'Enregistrer les réglages'}
            </button>
          </div>
        </form>

        {isUnavailable(error) ? (
          <Unavailable detail="L’enregistrement des réglages (PATCH /admin/api/guilds/{id}/settings) n’est pas exposé par ce serveur." />
        ) : error ? (
          <ErrorNotice error={error} title="Enregistrement impossible" />
        ) : null}
      </div>
    </Card>
  );
}
