(() => {
  const form = document.getElementById('pairing-form');
  const statusNode = document.getElementById('status');
  const submitButton = document.getElementById('submit-button');
  const guestButton = document.getElementById('guest-button');

  const serverUrlInput = document.getElementById('server-url');
  const pairingCodeInput = document.getElementById('pairing-code');
  let cachedConfig = null;

  const setStatus = (message, kind) => {
    statusNode.textContent = message;
    statusNode.className = kind || '';
  };

  const setBusy = (busy) => {
    submitButton.disabled = busy;
    guestButton.disabled = busy;
  };

  const normalizeServerUrl = (value) => `${value || ''}`.trim().replace(/\/+$/, '');

  const hasStoredPairing = (config) => {
    return !!(config?.serverUrl && config?.clientToken && config?.guildId);
  };

  const bootstrapDefaults = async () => {
    const config = await window.livechatOverlay.getConfig();
    cachedConfig = config;

    if (config?.serverUrl) {
      serverUrlInput.value = config.serverUrl;
    }
  };

  const consumePairing = async ({ guestMode }) => {
    await window.livechatOverlay.consumePairing({
      serverUrl: serverUrlInput.value.trim(),
      code: pairingCodeInput.value.trim(),
      guestMode: guestMode === true,
    });
  };

  form.addEventListener('submit', async (event) => {
    event.preventDefault();

    setBusy(true);
    setStatus('Appairage en cours...', '');

    try {
      await consumePairing({ guestMode: false });

      setStatus('Appairage réussi. Fermeture de la fenêtre...', 'success');
    } catch (error) {
      const reason = error?.message || 'Erreur inconnue';
      setStatus(`Échec de l'appairage: ${reason}`, 'error');
      setBusy(false);
    }
  });

  guestButton.addEventListener('click', async () => {
    setBusy(true);
    setStatus('Connexion invité en cours...', '');

    try {
      const config = cachedConfig || (await window.livechatOverlay.getConfig());
      const hasPairing = hasStoredPairing(config);
      const inputServerUrl = normalizeServerUrl(serverUrlInput.value);
      const storedServerUrl = normalizeServerUrl(config?.serverUrl);
      const canReuseStoredPairing = hasPairing && (!inputServerUrl || inputServerUrl === storedServerUrl);

      if (canReuseStoredPairing) {
        await window.livechatOverlay.setGuestMode({ enabled: true });
        setStatus('Mode invité activé. Fermeture de la fenêtre...', 'success');
        return;
      }

      if (!form.reportValidity()) {
        setStatus('URL serveur et code requis pour rejoindre en invité sans appairage existant.', 'error');
        setBusy(false);
        return;
      }

      await consumePairing({ guestMode: true });
      setStatus('Connexion invité réussie. Fermeture de la fenêtre...', 'success');
    } catch (error) {
      const reason = error?.message || 'Erreur inconnue';
      setStatus(`Échec de la connexion invité: ${reason}`, 'error');
      setBusy(false);
    }
  });

  bootstrapDefaults().catch((error) => {
    setStatus(`Impossible de charger la configuration: ${error?.message || error}`, 'error');
  });
})();
