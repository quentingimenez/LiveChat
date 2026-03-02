(() => {
  const form = document.getElementById('pairing-form');
  const statusNode = document.getElementById('status');
  const submitButton = document.getElementById('submit-button');
  const guestButton = document.getElementById('guest-button');

  const serverUrlInput = document.getElementById('server-url');
  const pairingCodeInput = document.getElementById('pairing-code');

  const setStatus = (message, kind) => {
    statusNode.textContent = message;
    statusNode.className = kind || '';
  };

  const setBusy = (busy) => {
    submitButton.disabled = busy;
    guestButton.disabled = busy;
  };

  const normalizeServerUrl = (value) => `${value || ''}`.trim().replace(/\/+$/, '');

  const bootstrapDefaults = async () => {
    const config = await window.livechatOverlay.getConfig();

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

  const parseGuestUrlInput = (rawValue) => {
    const parsedUrl = new URL(`${rawValue || ''}`.trim());
    const hashParams = new URLSearchParams(`${parsedUrl.hash || ''}`.replace(/^#/, ''));
    const queryParams = parsedUrl.searchParams;
    const baseServerUrl = normalizeServerUrl(`${parsedUrl.origin}${parsedUrl.pathname}`);
    const guestToken = `${hashParams.get('overlayGuestToken') || queryParams.get('overlayGuestToken') || ''}`.trim();
    const guestGuildId = `${hashParams.get('overlayGuestGuild') || queryParams.get('overlayGuestGuild') || queryParams.get('guildId') || ''}`.trim();
    const guestClientId = `${hashParams.get('overlayGuestClient') || queryParams.get('overlayGuestClient') || ''}`.trim();

    return {
      serverUrl: baseServerUrl,
      guestToken,
      guestGuildId,
      guestClientId,
    };
  };

  const validateGuestServerUrl = () => {
    serverUrlInput.setCustomValidity('');
    if (!serverUrlInput.reportValidity()) {
      return null;
    }

    try {
      return parseGuestUrlInput(serverUrlInput.value);
    } catch {
      setStatus('URL serveur invalide.', 'error');
      return null;
    }
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
      const guestInput = validateGuestServerUrl();
      if (!guestInput) {
        setBusy(false);
        return;
      }

      if (guestInput.guestToken && guestInput.guestGuildId) {
        await window.livechatOverlay.applyGuestConfig({
          serverUrl: guestInput.serverUrl,
          clientToken: guestInput.guestToken,
          guildId: guestInput.guestGuildId,
          clientId: guestInput.guestClientId || undefined,
        });
      } else {
        await window.livechatOverlay.connectGuest({
          serverUrl: guestInput.serverUrl,
          guildId: guestInput.guestGuildId || undefined,
        });
      }

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
