import { useEffect, useState } from 'preact/hooks';
import { DISCORD_LOGIN_URL } from '../api/client';
import type { AuthMethods } from '../api/types';
import { Icon } from '../components/icons';
import { describeError, loginErrorMessage } from '../errors';

export interface LoginScreenProps {
  methods: AuthMethods | null;
  errorCode?: string;
  serverError?: string | null;
  onSubmitSecret: (secret: string) => Promise<void>;
}

export function LoginScreen({ methods, errorCode, serverError, onSubmitSecret }: LoginScreenProps) {
  const [secret, setSecret] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    document.title = 'Connexion — LiveChat Administration';
  }, []);

  const discordAvailable = methods?.discord === true;
  const secretAvailable = methods?.secret === true;
  const methodsKnown = methods !== null;
  const noMethod = methodsKnown && !discordAvailable && !secretAvailable;
  const queryError = loginErrorMessage(errorCode);

  const submit = async (event: Event) => {
    event.preventDefault();
    if (submitting) {
      return;
    }
    setFormError(null);
    setSubmitting(true);
    try {
      await onSubmitSecret(secret);
    } catch (error) {
      setFormError(describeError(error));
    } finally {
      setSubmitting(false);
    }
  };

  const host = typeof window !== 'undefined' ? window.location.host : 'livechat';

  return (
    <div class="login">
      <section class="login-aside">
        <div class="brand">
          <svg aria-hidden="true" focusable="false">
            <use href="#i-logo" />
          </svg>
          <span>
            LiveChat
            <small>Administration du serveur</small>
          </span>
        </div>
        <div>
          <h1>La régie du serveur.</h1>
          <p class="lede">
            Guildes, overlays, accès, médias et réglages de toute l’instance. Réservé aux
            administrateurs du serveur LiveChat.
          </p>
        </div>
        <dl class="login-facts">
          <div>
            <dt>Instance</dt>
            <dd>{host}</dd>
          </div>
          <div>
            <dt>Version</dt>
            <dd>console</dd>
          </div>
          <div>
            <dt>État</dt>
            <dd>{serverError ? 'hors ligne' : 'opérationnel'}</dd>
          </div>
        </dl>
      </section>
      <main class="login-main" id="content">
        <div class="login-box">
          <h2>Connexion</h2>
          <p>Connecte-toi avec le compte Discord déclaré comme administrateur de ce serveur.</p>

          {queryError ? (
            <div class="login-error" role="alert">
              <Icon name="alert" />
              <span>{queryError}</span>
            </div>
          ) : null}
          {serverError ? (
            <div class="login-error" role="alert">
              <Icon name="alert" />
              <span>{serverError}</span>
            </div>
          ) : null}

          {methodsKnown && discordAvailable ? (
            <a class="btn-discord" href={DISCORD_LOGIN_URL}>
              <Icon name="discord" />
              Se connecter avec Discord
            </a>
          ) : null}

          {methodsKnown && discordAvailable && secretAvailable ? <div class="or">ou</div> : null}

          {methodsKnown && secretAvailable ? (
            <form onSubmit={submit}>
              <label for="secret">Secret d’administration</label>
              <input
                class="field is-after-label"
                id="secret"
                type="password"
                autocomplete="current-password"
                placeholder="ADMIN_UI_TOKEN"
                value={secret}
                onInput={(event) => setSecret(event.currentTarget.value)}
              />
              <p class="hint">
                Accès de secours, sans Discord : la valeur de <span class="mono">ADMIN_UI_TOKEN</span> du
                serveur.
              </p>
              <button class="btn is-block" type="submit" disabled={submitting || !secret}>
                {submitting ? 'Connexion…' : 'Se connecter avec le secret'}
              </button>
            </form>
          ) : null}

          {formError ? (
            <div class="login-error" role="alert">
              <Icon name="alert" />
              <span>{formError}</span>
            </div>
          ) : null}

          {noMethod ? (
            <p class="login-note is-err">
              <Icon name="info" />
              <span>
                Aucune méthode de connexion n’est disponible sur ce serveur. La console peut être
                désactivée ou ses accès pas encore configurés.
              </span>
            </p>
          ) : null}

          {!methodsKnown && !serverError ? (
            <p class="login-note">
              <Icon name="info" />
              <span>Vérification des méthodes de connexion…</span>
            </p>
          ) : null}

          <p class="login-note">
            <Icon name="shield" />
            <span>
              Session de 7 jours dans un cookie <span class="mono">HttpOnly</span>. Le secret n’est
              jamais stocké dans le navigateur ni placé dans une URL.
            </span>
          </p>
        </div>
      </main>
    </div>
  );
}
