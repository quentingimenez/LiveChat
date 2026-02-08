(() => {
  const mediaLayer = document.getElementById('media-layer');
  const textLayer = document.getElementById('text-layer');

  let overlayConfig = {
    serverUrl: null,
    clientToken: null,
    guildId: null,
    clientId: null,
    volume: 1,
    showText: true,
  };

  let resetTimer = null;
  let activeObjectUrl = null;

  const clearTimer = () => {
    if (resetTimer) {
      clearTimeout(resetTimer);
      resetTimer = null;
    }
  };

  const releaseObjectUrl = () => {
    if (activeObjectUrl) {
      URL.revokeObjectURL(activeObjectUrl);
      activeObjectUrl = null;
    }
  };

  const clearOverlay = () => {
    clearTimer();
    releaseObjectUrl();
    mediaLayer.innerHTML = '';
    textLayer.textContent = '';
    textLayer.style.display = 'none';
  };

  const applyText = (payload) => {
    if (!overlayConfig.showText) {
      textLayer.textContent = '';
      textLayer.style.display = 'none';
      return;
    }

    const textEnabled = payload?.text?.enabled;
    const value = (payload?.text?.value || '').trim();

    if (!textEnabled || !value) {
      textLayer.textContent = '';
      textLayer.style.display = 'none';
      return;
    }

    textLayer.textContent = value;
    textLayer.style.display = 'block';
  };

  const createMediaElement = (kind) => {
    if (kind === 'image') {
      return document.createElement('img');
    }

    if (kind === 'audio') {
      const audio = document.createElement('audio');
      audio.autoplay = true;
      audio.controls = false;
      audio.preload = 'auto';
      return audio;
    }

    const video = document.createElement('video');
    video.autoplay = true;
    video.controls = false;
    video.playsInline = true;
    video.preload = 'auto';
    return video;
  };

  const applyVolume = () => {
    const mediaElements = mediaLayer.querySelectorAll('audio,video');
    mediaElements.forEach((element) => {
      element.volume = overlayConfig.volume;
      element.muted = false;
    });
  };

  const fetchMediaBlobUrl = async (url) => {
    const response = await fetch(url, {
      headers: {
        Authorization: `Bearer ${overlayConfig.clientToken}`,
      },
    });

    if (!response.ok) {
      throw new Error(`media_fetch_failed_${response.status}`);
    }

    const blob = await response.blob();
    return URL.createObjectURL(blob);
  };

  const renderMedia = async (payload) => {
    if (!payload?.media) {
      return;
    }

    const mediaUrl = await fetchMediaBlobUrl(payload.media.url);
    activeObjectUrl = mediaUrl;

    const element = createMediaElement(payload.media.kind);
    element.src = mediaUrl;
    mediaLayer.appendChild(element);

    applyVolume();

    if (payload.media.kind !== 'image') {
      try {
        await element.play();
      } catch (error) {
        console.error('Autoplay failed:', error);
      }

      element.addEventListener(
        'ended',
        () => {
          clearOverlay();
        },
        { once: true },
      );
    }
  };

  const onPlay = async (payload) => {
    clearOverlay();

    applyText(payload);

    try {
      await renderMedia(payload);
    } catch (error) {
      console.error('Media render failed:', error);
      window.livechatOverlay.reportError({
        jobId: payload?.jobId || 'unknown-job',
        code: 'render_failed',
        message: error?.message || 'unknown_render_error',
      });
      clearOverlay();
      return;
    }

    if (payload?.durationSec) {
      resetTimer = setTimeout(() => {
        clearOverlay();
      }, payload.durationSec * 1000 + 100);
    }
  };

  const onSettings = (nextSettings) => {
    overlayConfig = {
      ...overlayConfig,
      ...nextSettings,
    };

    if (!overlayConfig.showText) {
      textLayer.textContent = '';
      textLayer.style.display = 'none';
    }

    applyVolume();
  };

  const bootstrap = async () => {
    overlayConfig = {
      ...overlayConfig,
      ...(await window.livechatOverlay.getConfig()),
    };

    window.livechatOverlay.onPlay(onPlay);
    window.livechatOverlay.onStop(() => clearOverlay());
    window.livechatOverlay.onSettings((settings) => onSettings(settings));

    window.livechatOverlay.rendererReady();
  };

  bootstrap().catch((error) => {
    console.error('Overlay bootstrap failed:', error);
  });
})();
