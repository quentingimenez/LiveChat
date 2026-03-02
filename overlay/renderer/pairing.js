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

  const consumePairing = async () => {
    await window.livechatOverlay.consumePairing({
      serverUrl: serverUrlInput.value.trim(),
      code: pairingCodeInput.value.trim(),
      guestMode: false,
    });
  };

  const validateGuestServerUrl = () => {
    serverUrlInput.setCustomValidity('');
    if (!serverUrlInput.reportValidity()) {
      return null;
    }

    return normalizeServerUrl(serverUrlInput.value);
  };

  form.addEventListener('submit', async (event) => {
    event.preventDefault();

    setBusy(true);
    setStatus('Appairage en cours...', '');

    try {
      await consumePairing();

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
      const inputServerUrl = validateGuestServerUrl();
      if (!inputServerUrl) {
        setBusy(false);
        return;
      }

      const config = cachedConfig || (await window.livechatOverlay.getConfig());
      const hasPairing = hasStoredPairing(config);
      const storedServerUrl = normalizeServerUrl(config?.serverUrl);
      if (!hasPairing) {
        setStatus("Mode invité indisponible: fais d'abord un appairage normal sur ce PC.", 'error');
        setBusy(false);
        return;
      }

      if (!storedServerUrl || inputServerUrl !== storedServerUrl) {
        setStatus(`Mode invité indisponible: utilise l'URL appairée (${storedServerUrl || 'inconnue'}).`, 'error');
        setBusy(false);
        return;
      }

      await window.livechatOverlay.setGuestMode({ enabled: true });
      setStatus('Mode invité activé. Fermeture de la fenêtre...', 'success');
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
