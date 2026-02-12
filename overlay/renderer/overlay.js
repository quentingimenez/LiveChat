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
  let twitterWidgetsPromise = null;

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

  const ensureTwitterWidgets = () => {
    if (window.twttr?.widgets?.load) {
      return Promise.resolve(window.twttr);
    }

    if (twitterWidgetsPromise) {
      return twitterWidgetsPromise;
    }

    twitterWidgetsPromise = new Promise((resolve, reject) => {
      const finish = () => {
        if (window.twttr?.widgets?.load) {
          resolve(window.twttr);
          return;
        }

        reject(new Error('twitter_widgets_unavailable'));
      };

      const existingScript = document.getElementById('twitter-widgets-js');

      if (existingScript) {
        setTimeout(finish, 0);
        return;
      }

      const script = document.createElement('script');
      script.id = 'twitter-widgets-js';
      script.async = true;
      script.src = 'https://platform.twitter.com/widgets.js';
      script.referrerPolicy = 'no-referrer-when-downgrade';
      script.onload = () => {
        if (window.twttr?.ready) {
          window.twttr.ready(() => finish());
          return;
        }

        setTimeout(finish, 0);
      };
      script.onerror = () => reject(new Error('twitter_widgets_load_failed'));
      document.head.appendChild(script);
    }).catch((error) => {
      twitterWidgetsPromise = null;
      throw error;
    });

    return twitterWidgetsPromise;
  };

  const renderTweetCard = (payload) => {
    const tweetCard = payload?.tweetCard;

    if (!tweetCard || typeof tweetCard.html !== 'string' || tweetCard.html.trim() === '') {
      return false;
    }

    if (typeof tweetCard.videoUrl === 'string' && tweetCard.videoUrl.trim() !== '') {
      const container = document.createElement('div');
      container.className = 'overlay-tweet-card overlay-tweet-card-with-video';

      const widgetContent = document.createElement('div');
      widgetContent.className = 'overlay-tweet-card-content overlay-tweet-widget';
      widgetContent.innerHTML = tweetCard.html.trim();

      const links = widgetContent.querySelectorAll('a[href]');
      links.forEach((link) => {
        link.setAttribute('target', '_blank');
        link.setAttribute('rel', 'noopener noreferrer');
      });

      container.appendChild(widgetContent);

      const inlineMedia = document.createElement('div');
      inlineMedia.className = 'overlay-tweet-inline-media';
      const video = document.createElement('video');
      video.className = 'overlay-tweet-inline-video';
      video.autoplay = true;
      video.controls = false;
      video.playsInline = true;
      video.preload = 'auto';
      video.muted = false;
      video.src = tweetCard.videoUrl;
      if (typeof tweetCard.videoIsVertical === 'boolean') {
        video.dataset.vertical = tweetCard.videoIsVertical ? '1' : '0';
      }
      inlineMedia.appendChild(video);
      container.appendChild(inlineMedia);

      mediaLayer.appendChild(container);

      ensureTwitterWidgets()
        .then((twitter) => {
          twitter.widgets.load(widgetContent);
        })
        .catch((error) => {
          console.warn('Twitter widgets fallback to raw HTML:', error?.message || error);
        });

      applyVolume();
      video
        .play()
        .catch((error) => {
          console.warn('Tweet inline video autoplay failed:', error?.message || error);
        });

      return true;
    }

    const container = document.createElement('div');
    container.className = 'overlay-tweet-card';

    const content = document.createElement('div');
    content.className = 'overlay-tweet-card-content';
    content.innerHTML = tweetCard.html.trim();

    const links = content.querySelectorAll('a[href]');
    links.forEach((link) => {
      link.setAttribute('target', '_blank');
      link.setAttribute('rel', 'noopener noreferrer');
    });

    container.appendChild(content);
    mediaLayer.appendChild(container);

    ensureTwitterWidgets()
      .then((twitter) => {
        twitter.widgets.load(content);
      })
      .catch((error) => {
        console.warn('Twitter widgets fallback to raw HTML:', error?.message || error);
      });

    return true;
  };

  const applyVolume = () => {
    const mediaElements = mediaLayer.querySelectorAll('audio,video');
    mediaElements.forEach((element) => {
      element.volume = overlayConfig.volume;
      element.muted = false;
    });
  };

  const buildAuthorizedMediaUrl = (url) => {
    const mediaUrl = new URL(url);
    mediaUrl.searchParams.set('token', overlayConfig.clientToken || '');
    return mediaUrl.toString();
  };

  const renderMedia = async (payload) => {
    if (!payload?.media) {
      return;
    }

    const mediaUrl = buildAuthorizedMediaUrl(payload.media.url);
    activeObjectUrl = null;

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
      if (!renderTweetCard(payload)) {
        await renderMedia(payload);
      }
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

    ensureTwitterWidgets().catch(() => undefined);

    window.livechatOverlay.rendererReady();
  };

  bootstrap().catch((error) => {
    console.error('Overlay bootstrap failed:', error);
  });
})();
