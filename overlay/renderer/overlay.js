(() => {
  const mediaLayer = document.getElementById('media-layer');
  const countdownLayer = document.getElementById('countdown-layer');
  const textLayer = document.getElementById('text-layer');

  let overlayConfig = {
    serverUrl: null,
    clientToken: null,
    guildId: null,
    clientId: null,
    volume: 1,
    showText: true,
  };
  const VOLUME_CURVE_GAMMA = 2.2;

  let resetTimer = null;
  let countdownTimer = null;
  let countdownRemainingMs = null;
  let countdownLastTickAtMs = null;
  let countdownPaused = false;
  let countdownAutoClear = false;
  let playbackSyncTimer = null;
  let activePlayableElement = null;
  let activePlayback = null;
  let activeObjectUrl = null;
  let twitterWidgetsPromise = null;

  const clearTimer = () => {
    if (resetTimer) {
      clearTimeout(resetTimer);
      resetTimer = null;
    }
  };

  const clearCountdownTimer = () => {
    if (countdownTimer) {
      clearInterval(countdownTimer);
      countdownTimer = null;
    }
  };

  const clearPlaybackSyncTimer = () => {
    if (playbackSyncTimer) {
      clearInterval(playbackSyncTimer);
      playbackSyncTimer = null;
    }
  };

  const formatRemainingTime = (remainingMs) => {
    const remainingSec = Math.max(0, Math.ceil(remainingMs / 1000));
    const minutes = Math.floor(remainingSec / 60);
    const seconds = remainingSec % 60;
    return `${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`;
  };

  const getRemainingCountdownMs = () => {
    if (typeof countdownRemainingMs !== 'number' || !Number.isFinite(countdownRemainingMs)) {
      return null;
    }

    return Math.max(0, Math.round(countdownRemainingMs));
  };

  const emitPlaybackState = (state) => {
    if (!activePlayback || !window.livechatOverlay?.reportPlaybackState) {
      return;
    }

    const payload = {
      jobId: activePlayback.jobId,
      state,
      remainingMs: getRemainingCountdownMs(),
    };

    window.livechatOverlay.reportPlaybackState(payload);
    console.debug(
      `[OVERLAY] playback-state sent (jobId: ${payload.jobId}, state: ${payload.state}, remainingMs: ${
        payload.remainingMs === null ? 'null' : payload.remainingMs
      })`,
    );
  };

  const getDerivedPlaybackState = () => {
    if (!activePlayback) {
      return null;
    }

    if (countdownPaused) {
      return 'paused';
    }

    if (activePlayableElement && typeof activePlayableElement.paused === 'boolean' && !activePlayableElement.ended) {
      return activePlayableElement.paused ? 'paused' : 'playing';
    }

    return activePlayback.state;
  };

  const setPlaybackState = (state) => {
    if (!activePlayback) {
      return;
    }

    if (activePlayback.state !== state) {
      activePlayback.state = state;
    }

    emitPlaybackState(activePlayback.state);
  };

  const startPlaybackSession = (jobId) => {
    if (typeof jobId !== 'string' || !jobId.trim()) {
      activePlayback = null;
      clearPlaybackSyncTimer();
      return;
    }

    activePlayback = {
      jobId: jobId.trim(),
      state: 'playing',
    };

    emitPlaybackState('playing');
    clearPlaybackSyncTimer();
    playbackSyncTimer = setInterval(() => {
      if (!activePlayback) {
        return;
      }

      const derivedState = getDerivedPlaybackState();
      if (derivedState && activePlayback.state !== derivedState) {
        activePlayback.state = derivedState;
      }

      emitPlaybackState(activePlayback.state);
    }, 1000);
  };

  const endPlaybackSession = () => {
    if (!activePlayback) {
      return;
    }

    const endedJobId = activePlayback.jobId;
    emitPlaybackState('ended');
    if (window.livechatOverlay?.reportPlaybackStop) {
      window.livechatOverlay.reportPlaybackStop({
        jobId: endedJobId,
      });
      console.debug(`[OVERLAY] playback-stop sent (jobId: ${endedJobId})`);
    }
    activePlayback = null;
    clearPlaybackSyncTimer();
  };

  const renderCountdown = () => {
    if (!countdownLayer) {
      return;
    }

    const remainingMs = getRemainingCountdownMs();
    if (remainingMs === null) {
      countdownLayer.textContent = '';
      countdownLayer.style.display = 'none';
      return;
    }

    const remainingDisplay = formatRemainingTime(remainingMs);

    countdownLayer.textContent = remainingDisplay;
    countdownLayer.style.display = 'flex';
  };

  const updateCountdown = () => {
    if (typeof countdownRemainingMs !== 'number' || !Number.isFinite(countdownRemainingMs)) {
      return;
    }

    const nowMs = Date.now();
    if (!countdownPaused && typeof countdownLastTickAtMs === 'number') {
      const elapsedMs = Math.max(0, nowMs - countdownLastTickAtMs);
      countdownRemainingMs = Math.max(0, countdownRemainingMs - elapsedMs);
    }
    countdownLastTickAtMs = nowMs;
    renderCountdown();

    if (countdownRemainingMs <= 0) {
      clearCountdownTimer();
      const isActiveMediaStillRunning =
        activePlayableElement &&
        typeof activePlayableElement.ended === 'boolean' &&
        !activePlayableElement.ended;

      if (countdownAutoClear || !isActiveMediaStillRunning) {
        clearOverlay();
      }
    }
  };

  const startCountdown = (durationSec, options = {}) => {
    if (!countdownLayer || typeof durationSec !== 'number' || !Number.isFinite(durationSec) || durationSec <= 0) {
      return;
    }

    clearCountdownTimer();
    countdownRemainingMs = durationSec * 1000;
    countdownLastTickAtMs = Date.now();
    countdownPaused = false;
    countdownAutoClear = options.autoClear === true;
    renderCountdown();
    countdownTimer = setInterval(updateCountdown, 200);
  };

  const pauseCountdown = () => {
    if (typeof countdownRemainingMs !== 'number' || countdownPaused) {
      return;
    }

    updateCountdown();
    countdownPaused = true;
  };

  const resumeCountdown = () => {
    if (typeof countdownRemainingMs !== 'number' || !countdownPaused) {
      return;
    }

    countdownPaused = false;
    countdownLastTickAtMs = Date.now();
  };

  const clearCountdown = () => {
    clearCountdownTimer();
    countdownRemainingMs = null;
    countdownLastTickAtMs = null;
    countdownPaused = false;
    countdownAutoClear = false;
    if (countdownLayer) {
      countdownLayer.textContent = '';
      countdownLayer.style.display = 'none';
    }
  };

  const releaseObjectUrl = () => {
    if (activeObjectUrl) {
      URL.revokeObjectURL(activeObjectUrl);
      activeObjectUrl = null;
    }
  };

  const clearOverlay = () => {
    endPlaybackSession();
    clearTimer();
    clearCountdown();
    activePlayableElement = null;
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

  const videoHasAudioTrack = (video) => {
    const candidate = video;

    if (typeof candidate.mozHasAudio === 'boolean') {
      return candidate.mozHasAudio;
    }

    if (candidate.audioTracks && typeof candidate.audioTracks.length === 'number') {
      return candidate.audioTracks.length > 0;
    }

    if (typeof candidate.webkitAudioDecodedByteCount === 'number') {
      return candidate.webkitAudioDecodedByteCount > 0;
    }

    return false;
  };

  const playVideoWithRetry = (video, attempts = 4, delayMs = 250) => {
    const attemptPlay = (remainingAttempts) => {
      video.play().catch((error) => {
        if (remainingAttempts <= 0) {
          console.warn('Tweet inline video autoplay failed:', error?.message || error);
          return;
        }

        setTimeout(() => attemptPlay(remainingAttempts - 1), delayMs);
      });
    };

    attemptPlay(attempts);
  };

  const getDurationSec = (value) => {
    if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
      return null;
    }

    return value;
  };

  const clamp01 = (value) => {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
      return 1;
    }

    return Math.min(1, Math.max(0, value));
  };

  const toPerceptualGain = (linearVolume) => {
    const normalized = clamp01(linearVolume);
    return Math.pow(normalized, VOLUME_CURVE_GAMMA);
  };

  const configureInlineVideoLooping = (videos) => {
    if (!Array.isArray(videos) || videos.length < 2) {
      return;
    }

    const configuredDurations = videos.map((video) => {
      const raw = Number.parseFloat(video.dataset.configDurationSec || '');
      return getDurationSec(raw);
    });

    const applyLoopPolicy = () => {
      const measuredDurations = videos.map((video, index) => {
        const measured = getDurationSec(video.duration);
        return measured ?? configuredDurations[index] ?? null;
      });

      const knownDurations = measuredDurations.filter((durationSec) => typeof durationSec === 'number');
      let longestIndex = 0;

      if (knownDurations.length > 0) {
        const longestDuration = Math.max(...knownDurations);
        longestIndex = measuredDurations.findIndex((durationSec) => durationSec === longestDuration);
        if (longestIndex < 0) {
          longestIndex = 0;
        }
      }

      videos.forEach((video, index) => {
        const durationSec = measuredDurations[index];
        const shouldLoop =
          knownDurations.length === 0
            ? index > 0
            : index !== longestIndex && (durationSec === null || durationSec + 0.25 < measuredDurations[longestIndex]);

        video.loop = shouldLoop;
        video.dataset.loopWanted = shouldLoop ? '1' : '0';
      });
    };

    videos.forEach((video) => {
      video.addEventListener('loadedmetadata', applyLoopPolicy);
      video.addEventListener('ended', () => {
        if (video.dataset.loopWanted !== '1') {
          return;
        }

        video.currentTime = 0;
        playVideoWithRetry(video, 2, 120);
      });
    });

    applyLoopPolicy();
  };

  const ensureInlineVideoAudioFallback = (videos) => {
    if (!Array.isArray(videos) || videos.length < 2) {
      return;
    }

    const primary = videos[0];
    if (!(primary instanceof HTMLVideoElement)) {
      return;
    }

    const tryPromoteAudibleVideo = () => {
      if (videoHasAudioTrack(primary)) {
        return;
      }

      const replacement = videos.slice(1).find((video) => {
        if (!(video instanceof HTMLVideoElement)) {
          return false;
        }

        return videoHasAudioTrack(video);
      });

      if (!replacement) {
        return;
      }

      primary.dataset.forceMuted = '1';
      replacement.dataset.forceMuted = '0';
      applyVolume();
    };

    setTimeout(tryPromoteAudibleVideo, 1000);
    setTimeout(tryPromoteAudibleVideo, 2200);
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

    const inlineVideos = [];
    const inlineVideoSeen = new Set();

    const pushInlineVideo = (candidate) => {
      if (!candidate || typeof candidate !== 'object') {
        return;
      }

      const rawUrl = typeof candidate.url === 'string' ? candidate.url.trim() : '';
      if (!rawUrl) {
        return;
      }

      const key = `${typeof candidate.sourceStatusId === 'string' ? candidate.sourceStatusId : 'source-unknown'}:${rawUrl}`;
      if (inlineVideoSeen.has(key)) {
        return;
      }

      inlineVideoSeen.add(key);
      inlineVideos.push({
        url: rawUrl,
        isVertical: typeof candidate.isVertical === 'boolean' ? candidate.isVertical : null,
        sourceStatusId: typeof candidate.sourceStatusId === 'string' ? candidate.sourceStatusId : null,
        durationSec:
          typeof candidate.durationSec === 'number' && Number.isFinite(candidate.durationSec) && candidate.durationSec > 0
            ? candidate.durationSec
            : null,
      });
    };

    if (Array.isArray(tweetCard.videos)) {
      tweetCard.videos.forEach((video) => {
        pushInlineVideo(video);
      });
    }

    if (inlineVideos.length === 0 && typeof tweetCard.videoUrl === 'string' && tweetCard.videoUrl.trim() !== '') {
      pushInlineVideo({
        url: tweetCard.videoUrl,
        isVertical: typeof tweetCard.videoIsVertical === 'boolean' ? tweetCard.videoIsVertical : null,
        sourceStatusId: null,
      });
    }

    const videosToRender = inlineVideos.slice(0, 2);

    if (videosToRender.length > 0) {
      const container = document.createElement('div');
      container.className = 'overlay-tweet-card overlay-tweet-card-with-video';
      if (videosToRender.length > 1) {
        container.classList.add('overlay-tweet-card-with-multi-video');
      }

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
      inlineMedia.dataset.count = `${videosToRender.length}`;
      const currentStatusId =
        typeof tweetCard.currentStatusId === 'string' && tweetCard.currentStatusId.trim() !== ''
          ? tweetCard.currentStatusId.trim()
          : null;
      const hasCurrentTweetVideo =
        !!currentStatusId &&
        videosToRender.some((video) => typeof video.sourceStatusId === 'string' && video.sourceStatusId === currentStatusId);
      const hasOriginalTweetVideo =
        !!currentStatusId &&
        videosToRender.some(
          (video) =>
            typeof video.sourceStatusId === 'string' &&
            video.sourceStatusId.trim() !== '' &&
            video.sourceStatusId !== currentStatusId,
        );
      const hasReplyContext = hasCurrentTweetVideo && hasOriginalTweetVideo;

      videosToRender.forEach((inlineVideo, index) => {
        const inlineItem = document.createElement('div');
        inlineItem.className = 'overlay-tweet-inline-item';

        const roleLabel = document.createElement('div');
        roleLabel.className = 'overlay-tweet-inline-label';
        if (hasReplyContext && currentStatusId && inlineVideo.sourceStatusId) {
          roleLabel.textContent = inlineVideo.sourceStatusId === currentStatusId ? 'Reponse' : 'Tweet original';
        } else if (videosToRender.length === 1) {
          roleLabel.textContent = 'Video';
        } else if (index === 0) {
          roleLabel.textContent = 'Video 1';
        } else {
          roleLabel.textContent = 'Video 2';
        }
        inlineItem.appendChild(roleLabel);

        const video = document.createElement('video');
        video.className = 'overlay-tweet-inline-video';
        video.autoplay = true;
        video.controls = false;
        video.playsInline = true;
        video.preload = 'auto';
        video.dataset.configDurationSec =
          typeof inlineVideo.durationSec === 'number' && Number.isFinite(inlineVideo.durationSec) && inlineVideo.durationSec > 0
            ? `${inlineVideo.durationSec}`
            : '';
        video.muted = true;
        video.dataset.forceMuted = index > 0 ? '1' : '0';
        video.src = inlineVideo.url;
        if (typeof inlineVideo.isVertical === 'boolean') {
          video.dataset.vertical = inlineVideo.isVertical ? '1' : '0';
        }
        inlineItem.appendChild(video);
        inlineMedia.appendChild(inlineItem);
      });
      container.appendChild(inlineMedia);

      mediaLayer.appendChild(container);

      ensureTwitterWidgets()
        .then((twitter) => {
          twitter.widgets.load(widgetContent);
        })
        .catch((error) => {
          console.warn('Twitter widgets fallback to raw HTML:', error?.message || error);
        });

      const inlineVideoElements = inlineMedia.querySelectorAll('video');
      configureInlineVideoLooping(Array.from(inlineVideoElements));
      inlineVideoElements.forEach((video) => {
        playVideoWithRetry(video);
      });
      setTimeout(() => {
        applyVolume();
      }, 220);
      ensureInlineVideoAudioFallback(Array.from(inlineVideoElements));

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
    const perceptualGain = toPerceptualGain(overlayConfig.volume);
    const mediaElements = mediaLayer.querySelectorAll('audio,video');
    mediaElements.forEach((element) => {
      const forceMuted = element.dataset?.forceMuted === '1';
      if (forceMuted) {
        element.volume = 0;
        element.muted = true;
        return;
      }

      element.volume = perceptualGain;
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
      activePlayableElement = element;

      element.addEventListener('playing', () => {
        resumeCountdown();
        setPlaybackState('playing');
      });

      element.addEventListener('pause', () => {
        if (element.ended) {
          return;
        }

        pauseCountdown();
        setPlaybackState('paused');
      });

      try {
        await element.play();
      } catch (error) {
        console.error('Autoplay failed:', error);
        pauseCountdown();
        setPlaybackState('paused');
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
    startPlaybackSession(payload?.jobId || null);

    applyOverlayInfo(payload);

    const hasStandaloneMedia = !!payload?.media;
    const shouldAutoClearByTimer = !hasStandaloneMedia || payload.media.kind === 'image';

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
      startCountdown(payload.durationSec, { autoClear: shouldAutoClearByTimer });

      if (shouldAutoClearByTimer) {
        resetTimer = setTimeout(() => {
          clearOverlay();
        }, payload.durationSec * 1000 + 100);
      }
    } else {
      clearCountdown();
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
