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
    textLayer.innerHTML = '';
    textLayer.style.display = 'none';
  };

  const createAvatarNode = (authorName, authorImage) => {
    if (authorImage) {
      const avatarImage = document.createElement('img');
      avatarImage.className = 'overlay-author-avatar';
      avatarImage.src = authorImage;
      avatarImage.alt = authorName;
      avatarImage.referrerPolicy = 'no-referrer';
      avatarImage.addEventListener('error', () => {
        avatarImage.replaceWith(createAvatarNode(authorName, null));
      });
      return avatarImage;
    }

    const fallback = document.createElement('div');
    fallback.className = 'overlay-author-avatar-fallback';
    fallback.textContent = (authorName || '?').trim().charAt(0).toUpperCase() || '?';
    return fallback;
  };

  const applyOverlayInfo = (payload) => {
    if (!overlayConfig.showText) {
      textLayer.innerHTML = '';
      textLayer.style.display = 'none';
      return;
    }

    const textEnabled = payload?.text?.enabled === true;
    const textValue = (payload?.text?.value || '').trim();
    const authorEnabled = payload?.author?.enabled === true;
    const authorName = (payload?.author?.name || '').trim();
    const authorImage = typeof payload?.author?.image === 'string' ? payload.author.image.trim() : null;
    const showAuthor = authorEnabled && authorName !== '';
    const showText = textEnabled && textValue !== '';

    if (!showAuthor && !showText) {
      textLayer.innerHTML = '';
      textLayer.style.display = 'none';
      return;
    }

    textLayer.innerHTML = '';

    const metaNode = document.createElement('div');
    metaNode.className = 'overlay-meta';

    if (showAuthor) {
      const authorNode = document.createElement('div');
      authorNode.className = 'overlay-author';

      authorNode.appendChild(createAvatarNode(authorName, authorImage));

      const authorNameNode = document.createElement('div');
      authorNameNode.className = 'overlay-author-name';
      authorNameNode.textContent = authorName;
      authorNode.appendChild(authorNameNode);

      metaNode.appendChild(authorNode);
    }

    if (showText) {
      const textNode = document.createElement('div');
      textNode.className = 'overlay-text-value';
      textNode.textContent = textValue;
      metaNode.appendChild(textNode);
    }

    textLayer.appendChild(metaNode);
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

    applyOverlayInfo(payload);

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
      textLayer.innerHTML = '';
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
