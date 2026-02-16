(() => {
  const form = document.getElementById('pairing-form');
  const statusNode = document.getElementById('status');
  const submitButton = document.getElementById('submit-button');

  const serverUrlInput = document.getElementById('server-url');
  const pairingCodeInput = document.getElementById('pairing-code');

  const setStatus = (message, kind) => {
    statusNode.textContent = message;
    statusNode.className = kind || '';
  };

  const bootstrapDefaults = async () => {
    const config = await window.livechatOverlay.getConfig();

    if (config?.serverUrl) {
      serverUrlInput.value = config.serverUrl;
    }
  };

  form.addEventListener('submit', async (event) => {
    event.preventDefault();

    submitButton.disabled = true;
    setStatus('Appairage en cours...', '');

    try {
      await window.livechatOverlay.consumePairing({
        serverUrl: serverUrlInput.value.trim(),
        code: pairingCodeInput.value.trim(),
      });

      setStatus('Appairage réussi. Fermeture de la fenêtre...', 'success');
    } catch (error) {
      const reason = error?.message || 'Erreur inconnue';
      setStatus(`Échec de l'appairage: ${reason}`, 'error');
      submitButton.disabled = false;
    }
  });

  bootstrapDefaults().catch((error) => {
    setStatus(`Impossible de charger la configuration: ${error?.message || error}`, 'error');
  });
})();
