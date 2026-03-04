import type { PairingDom } from './pairing-dom';

function getErrorMessage(error: unknown): string {
  if (error instanceof Error && typeof error.message === 'string' && error.message.trim().length > 0) {
    return error.message;
  }

  if (typeof error === 'string' && error.trim().length > 0) {
    return error;
  }

  return 'Erreur inconnue';
}

export function initializePairingController(dom: PairingDom): void {
  async function bootstrapDefaults(): Promise<void> {
    const config = await window.livechatOverlay.getConfig();

    if (config?.serverUrl) {
      dom.serverUrlInput.value = config.serverUrl;
    }
  }

  async function consumePairing(): Promise<void> {
    await window.livechatOverlay.consumePairing({
      serverUrl: dom.serverUrlInput.value.trim(),
      code: dom.pairingCodeInput.value.trim()
    });
  }

  dom.form.addEventListener('submit', (event) => {
    event.preventDefault();

    void (async () => {
      dom.setBusy(true);
      dom.setStatus('Appairage en cours...');

      try {
        await consumePairing();
        dom.setStatus('Appairage réussi. Fermeture de la fenêtre...', 'success');
      } catch (error) {
        dom.setStatus(`Échec de l'appairage: ${getErrorMessage(error)}`, 'error');
        dom.setBusy(false);
      }
    })();
  });

  bootstrapDefaults().catch((error) => {
    dom.setStatus(`Impossible de charger la configuration: ${getErrorMessage(error)}`, 'error');
  });
}
