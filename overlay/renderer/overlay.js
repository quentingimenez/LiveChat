(() => {
  const overlayRoot = document.getElementById('overlay-root');
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
  let activeMediaFrame = null;
  let activeMediaContent = null;
  let activeMediaHeaderLeft = null;
  let activeMediaHeaderRight = null;
  let activeMediaFooter = null;
  let mediaHeaderLayoutRaf = null;

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

  const clearMediaHeaderLayoutRaf = () => {
    if (mediaHeaderLayoutRaf === null) {
      return;
    }

    if (typeof window.cancelAnimationFrame === 'function') {
      window.cancelAnimationFrame(mediaHeaderLayoutRaf);
    } else {
      clearTimeout(mediaHeaderLayoutRaf);
    }
    mediaHeaderLayoutRaf = null;
  };

  const syncMediaHeaderLayout = () => {
    if (
      !(activeMediaFrame instanceof HTMLElement) ||
      !(activeMediaHeaderLeft instanceof HTMLElement) ||
      !(activeMediaHeaderRight instanceof HTMLElement)
    ) {
      return;
    }

    const mediaNode = activeMediaContent?.firstElementChild;
    const mediaWidth =
      mediaNode instanceof HTMLElement
        ? Math.max(0, Math.round(mediaNode.getBoundingClientRect().width))
        : Math.max(0, Math.round(activeMediaFrame.getBoundingClientRect().width));

    const leftWidth = activeMediaHeaderLeft.scrollWidth;
    const rightWidth = activeMediaHeaderRight.scrollWidth;
    const minGapPx = 14;
    const overflowPx = mediaWidth > 0 ? leftWidth + rightWidth + minGapPx - mediaWidth : 0;

    if (overflowPx > 0) {
      const leftShiftPx = Math.ceil(overflowPx / 2);
      const rightShiftPx = Math.floor(overflowPx / 2);
      activeMediaFrame.style.setProperty('--header-left-shift', `${leftShiftPx}px`);
      activeMediaFrame.style.setProperty('--header-right-shift', `${rightShiftPx}px`);
      return;
    }

    activeMediaFrame.style.setProperty('--header-left-shift', '0px');
    activeMediaFrame.style.setProperty('--header-right-shift', '0px');
  };

  const scheduleMediaHeaderLayout = () => {
    if (mediaHeaderLayoutRaf !== null) {
      return;
    }

    if (typeof window.requestAnimationFrame === 'function') {
      mediaHeaderLayoutRaf = window.requestAnimationFrame(() => {
        mediaHeaderLayoutRaf = null;
        syncMediaHeaderLayout();
      });
      return;
    }

    mediaHeaderLayoutRaf = setTimeout(() => {
      mediaHeaderLayoutRaf = null;
      syncMediaHeaderLayout();
    }, 0);
  };

  const ensureMediaFrame = () => {
    if (!(mediaLayer instanceof HTMLElement)) {
      return null;
    }

    if (
      activeMediaFrame instanceof HTMLElement &&
      activeMediaContent instanceof HTMLElement &&
      activeMediaHeaderLeft instanceof HTMLElement &&
      activeMediaHeaderRight instanceof HTMLElement
    ) {
      if (countdownLayer instanceof HTMLElement && countdownLayer.parentElement !== activeMediaHeaderRight) {
        activeMediaHeaderRight.appendChild(countdownLayer);
      }
      return activeMediaContent;
    }

    const frame = document.createElement('div');
    frame.className = 'overlay-media-frame';
    frame.style.setProperty('--header-left-shift', '0px');
    frame.style.setProperty('--header-right-shift', '0px');

    const header = document.createElement('div');
    header.className = 'overlay-media-header';

    const headerLeft = document.createElement('div');
    headerLeft.className = 'overlay-media-header-left';
    const headerRight = document.createElement('div');
    headerRight.className = 'overlay-media-header-right';

    header.appendChild(headerLeft);
    header.appendChild(headerRight);

    const content = document.createElement('div');
    content.className = 'overlay-media-content';
    const footer = document.createElement('div');
    footer.className = 'overlay-media-footer';
    footer.style.display = 'none';

    frame.appendChild(header);
    frame.appendChild(content);
    frame.appendChild(footer);
    mediaLayer.appendChild(frame);

    activeMediaFrame = frame;
    activeMediaContent = content;
    activeMediaHeaderLeft = headerLeft;
    activeMediaHeaderRight = headerRight;
    activeMediaFooter = footer;

    if (countdownLayer instanceof HTMLElement) {
      headerRight.appendChild(countdownLayer);
    }

    scheduleMediaHeaderLayout();
    return content;
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
      scheduleMediaHeaderLayout();
      return;
    }

    const remainingDisplay = formatRemainingTime(remainingMs);

    countdownLayer.textContent = remainingDisplay;
    countdownLayer.style.display = 'flex';
    scheduleMediaHeaderLayout();
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
    scheduleMediaHeaderLayout();
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
    clearMediaHeaderLayoutRaf();
    activePlayableElement = null;
    releaseObjectUrl();
    mediaLayer.innerHTML = '';
    activeMediaFrame = null;
    activeMediaContent = null;
    activeMediaHeaderLeft = null;
    activeMediaHeaderRight = null;
    activeMediaFooter = null;
    if (
      overlayRoot instanceof HTMLElement &&
      countdownLayer instanceof HTMLElement &&
      countdownLayer.parentElement !== overlayRoot
    ) {
      overlayRoot.appendChild(countdownLayer);
    }
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

  const resolveAuthorInfo = (payload) => {
    const authorEnabled = payload?.author?.enabled === true;
    const authorName = (payload?.author?.name || '').trim();
    const authorImage = typeof payload?.author?.image === 'string' ? payload.author.image.trim() : null;
    const showAuthor = authorEnabled && authorName !== '';

    return {
      showAuthor,
      authorName,
      authorImage,
    };
  };

  const applyMediaHeaderAuthor = (payload, options = {}) => {
    if (!(activeMediaHeaderLeft instanceof HTMLElement)) {
      return;
    }

    activeMediaHeaderLeft.innerHTML = '';

    if (!overlayConfig.showText || options.enabled !== true) {
      scheduleMediaHeaderLayout();
      return;
    }

    const authorInfo = resolveAuthorInfo(payload);
    if (!authorInfo.showAuthor) {
      scheduleMediaHeaderLayout();
      return;
    }

    const authorNode = document.createElement('div');
    authorNode.className = 'overlay-author';

    authorNode.appendChild(createAvatarNode(authorInfo.authorName, authorInfo.authorImage));

    const authorNameNode = document.createElement('div');
    authorNameNode.className = 'overlay-author-name';
    authorNameNode.textContent = authorInfo.authorName;
    authorNode.appendChild(authorNameNode);

    activeMediaHeaderLeft.appendChild(authorNode);
    scheduleMediaHeaderLayout();
  };

  const applyMediaFooterText = (payload, options = {}) => {
    if (!(activeMediaFooter instanceof HTMLElement)) {
      return;
    }

    activeMediaFooter.innerHTML = '';

    if (!overlayConfig.showText || options.enabled !== true) {
      activeMediaFooter.style.display = 'none';
      return;
    }

    const textEnabled = payload?.text?.enabled === true;
    const textValue = (payload?.text?.value || '').trim();
    const showText = textEnabled && textValue !== '';

    if (!showText) {
      activeMediaFooter.style.display = 'none';
      return;
    }

    const metaNode = document.createElement('div');
    metaNode.className = 'overlay-meta overlay-media-footer-meta';

    const textNode = document.createElement('div');
    textNode.className = 'overlay-text-value';
    textNode.textContent = textValue;
    metaNode.appendChild(textNode);

    activeMediaFooter.appendChild(metaNode);
    activeMediaFooter.style.display = 'flex';
  };

  const applyOverlayInfo = (payload, options = {}) => {
    if (!overlayConfig.showText) {
      textLayer.innerHTML = '';
      textLayer.style.display = 'none';
      return;
    }

    const showAuthorInline = options.showAuthorInline === true;
    const attachTextToMedia = options.attachTextToMedia === true;
    const textEnabled = payload?.text?.enabled === true;
    const textValue = (payload?.text?.value || '').trim();
    const authorInfo = resolveAuthorInfo(payload);
    const showText = textEnabled && textValue !== '';
    const showTextInOverlayLayer = showText && !attachTextToMedia;

    if (!(showAuthorInline && authorInfo.showAuthor) && !showTextInOverlayLayer) {
      textLayer.innerHTML = '';
      textLayer.style.display = 'none';
      return;
    }

    textLayer.innerHTML = '';

    const metaNode = document.createElement('div');
    metaNode.className = 'overlay-meta';

    if (showAuthorInline && authorInfo.showAuthor) {
      const authorNode = document.createElement('div');
      authorNode.className = 'overlay-author';

      authorNode.appendChild(createAvatarNode(authorInfo.authorName, authorInfo.authorImage));

      const authorNameNode = document.createElement('div');
      authorNameNode.className = 'overlay-author-name';
      authorNameNode.textContent = authorInfo.authorName;
      authorNode.appendChild(authorNameNode);

      metaNode.appendChild(authorNode);
    }

    if (showTextInOverlayLayer) {
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

  const getStartOffsetSec = (value) => {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
      return null;
    }

    const normalized = Math.floor(value);
    return normalized > 0 ? normalized : null;
  };

  const parseStartOffsetFromMediaUrl = (rawUrl) => {
    if (typeof rawUrl !== 'string' || !rawUrl.trim()) {
      return null;
    }

    try {
      const parsed = new URL(rawUrl);
      const queryValue = parsed.searchParams.get('startOffsetSec');
      if (queryValue && /^\d+$/.test(queryValue)) {
        const parsedQueryValue = Number.parseInt(queryValue, 10);
        if (Number.isFinite(parsedQueryValue) && parsedQueryValue > 0) {
          return parsedQueryValue;
        }
      }

      const hash = parsed.hash.replace(/^#/, '').trim().toLowerCase();
      if (!hash) {
        return null;
      }

      if (/^\d+$/.test(hash)) {
        const parsedHashValue = Number.parseInt(hash, 10);
        return Number.isFinite(parsedHashValue) && parsedHashValue > 0 ? parsedHashValue : null;
      }

      const hashParams = new URLSearchParams(hash);
      const hashTValue = hashParams.get('t') || hashParams.get('start');
      if (hashTValue && /^\d+$/.test(hashTValue)) {
        const parsedHashTValue = Number.parseInt(hashTValue, 10);
        return Number.isFinite(parsedHashTValue) && parsedHashTValue > 0 ? parsedHashTValue : null;
      }

      return null;
    } catch {
      return null;
    }
  };

  const resolveStartOffsetSec = (mediaPayload) => {
    const directOffset = getStartOffsetSec(mediaPayload?.startOffsetSec);
    if (directOffset !== null) {
      return directOffset;
    }

    return parseStartOffsetFromMediaUrl(mediaPayload?.url);
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

      const targetContainer = ensureMediaFrame() || mediaLayer;
      if (!(targetContainer instanceof HTMLElement)) {
        return false;
      }

      targetContainer.appendChild(container);
      scheduleMediaHeaderLayout();

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
        video.addEventListener('loadedmetadata', () => {
          scheduleMediaHeaderLayout();
        });
      });
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
    const targetContainer = ensureMediaFrame() || mediaLayer;
    if (!(targetContainer instanceof HTMLElement)) {
      return false;
    }

    targetContainer.appendChild(container);
    scheduleMediaHeaderLayout();

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

  const toRedactedMediaUrl = (value) => {
    if (typeof value !== 'string' || value.trim() === '') {
      return 'unknown-url';
    }

    try {
      const parsed = new URL(value);
      if (parsed.searchParams.has('token')) {
        parsed.searchParams.set('token', '***');
      }
      return parsed.toString();
    } catch {
      return value;
    }
  };

  const hasTokenInMediaUrl = (value) => {
    if (typeof value !== 'string' || value.trim() === '') {
      return false;
    }

    try {
      const parsed = new URL(value);
      const token = parsed.searchParams.get('token');
      return typeof token === 'string' && token.trim() !== '';
    } catch {
      return false;
    }
  };

  const getHtmlMediaErrorLabel = (code) => {
    switch (code) {
      case 1:
        return 'MEDIA_ERR_ABORTED';
      case 2:
        return 'MEDIA_ERR_NETWORK';
      case 3:
        return 'MEDIA_ERR_DECODE';
      case 4:
        return 'MEDIA_ERR_SRC_NOT_SUPPORTED';
      default:
        return 'MEDIA_ERR_UNKNOWN';
    }
  };

  const probeMediaHttpStatus = async (params) => {
    const mediaUrl = typeof params?.mediaUrl === 'string' ? params.mediaUrl : '';
    const mediaKind = typeof params?.mediaKind === 'string' ? params.mediaKind : 'unknown';
    const reason = typeof params?.reason === 'string' ? params.reason : 'unknown';
    const jobId = typeof params?.jobId === 'string' && params.jobId.trim() ? params.jobId.trim() : 'unknown-job';
    const redactedUrl = toRedactedMediaUrl(mediaUrl);

    if (!mediaUrl) {
      return;
    }

    const abortController = typeof AbortController === 'function' ? new AbortController() : null;
    const probeTimeout = setTimeout(() => {
      abortController?.abort();
    }, 6000);

    try {
      const response = await fetch(mediaUrl, {
        method: 'GET',
        headers: {
          Range: 'bytes=0-0',
        },
        cache: 'no-store',
        signal: abortController?.signal,
      });
      const statusCode = response.status;
      const statusHint = statusCode === 401 || statusCode === 403 ? 'auth_failed' : 'non_auth_error';

      console.warn(
        `[OVERLAY] Media probe (${reason}) jobId=${jobId} kind=${mediaKind} status=${statusCode} hint=${statusHint} url=${redactedUrl}`,
      );

      if (response.body && typeof response.body.cancel === 'function') {
        response.body.cancel().catch(() => undefined);
      }
    } catch (error) {
      const message =
        error && typeof error === 'object' && error.name === 'AbortError'
          ? 'probe_timeout'
          : error?.message || 'probe_request_failed';

      console.warn(`[OVERLAY] Media probe failed (${reason}) jobId=${jobId} kind=${mediaKind} error=${message} url=${redactedUrl}`);
    } finally {
      clearTimeout(probeTimeout);
    }
  };

  const attachMediaDiagnostics = (element, params) => {
    if (!(element instanceof HTMLElement)) {
      return;
    }

    const mediaUrl = typeof params?.mediaUrl === 'string' ? params.mediaUrl : '';
    const mediaKind = typeof params?.mediaKind === 'string' ? params.mediaKind : 'unknown';
    const jobId = typeof params?.jobId === 'string' && params.jobId.trim() ? params.jobId.trim() : 'unknown-job';
    const redactedUrl = toRedactedMediaUrl(mediaUrl);
    const tokenPresent = hasTokenInMediaUrl(mediaUrl);
    let hasProbedStatus = false;

    const probeOnce = (reason) => {
      if (hasProbedStatus) {
        return;
      }

      hasProbedStatus = true;
      void probeMediaHttpStatus({
        mediaUrl,
        mediaKind,
        reason,
        jobId,
      });
    };

    element.addEventListener('error', () => {
      if (element instanceof HTMLMediaElement) {
        const mediaErrorCode = typeof element.error?.code === 'number' ? element.error.code : 0;
        const mediaError = getHtmlMediaErrorLabel(mediaErrorCode);

        console.warn(
          `[OVERLAY] Media element error jobId=${jobId} kind=${mediaKind} error=${mediaError} tokenPresent=${
            tokenPresent ? 'yes' : 'no'
          } url=${redactedUrl}`,
        );
      } else {
        console.warn(
          `[OVERLAY] Image element error jobId=${jobId} kind=${mediaKind} tokenPresent=${
            tokenPresent ? 'yes' : 'no'
          } url=${redactedUrl}`,
        );
      }

      probeOnce('error');
    });

    if (element instanceof HTMLMediaElement) {
      element.addEventListener('stalled', () => {
        console.warn(
          `[OVERLAY] Media element stalled jobId=${jobId} kind=${mediaKind} readyState=${element.readyState} networkState=${element.networkState} tokenPresent=${
            tokenPresent ? 'yes' : 'no'
          } url=${redactedUrl}`,
        );
        probeOnce('stalled');
      });
    }
  };

  const applyMediaStartOffset = (element, startOffsetSec) => {
    if (!element || typeof element.currentTime !== 'number') {
      return;
    }

    const resolvedOffset = getStartOffsetSec(startOffsetSec);
    if (resolvedOffset === null) {
      return;
    }

    const seekToOffset = () => {
      const durationSec = Number.isFinite(element.duration) && element.duration > 0 ? element.duration : null;
      const targetSec =
        durationSec === null ? resolvedOffset : Math.min(resolvedOffset, Math.max(0, durationSec - 0.05));

      if (!Number.isFinite(targetSec) || targetSec <= 0) {
        return;
      }

      try {
        element.currentTime = targetSec;
      } catch (error) {
        console.warn('Media offset seek failed:', error?.message || error);
      }
    };

    if (typeof element.readyState === 'number' && element.readyState >= 1) {
      seekToOffset();
      return;
    }

    element.addEventListener('loadedmetadata', seekToOffset, { once: true });
  };

  const renderMedia = async (payload) => {
    if (!payload?.media) {
      return;
    }

    const mediaUrl = buildAuthorizedMediaUrl(payload.media.url);
    activeObjectUrl = null;

    const element = createMediaElement(payload.media.kind);
    attachMediaDiagnostics(element, {
      mediaUrl,
      mediaKind: payload.media.kind,
      jobId: payload?.jobId || 'unknown-job',
    });
    element.src = mediaUrl;
    const targetContainer = ensureMediaFrame() || mediaLayer;
    if (!(targetContainer instanceof HTMLElement)) {
      return;
    }

    targetContainer.appendChild(element);
    scheduleMediaHeaderLayout();
    if (payload.media.kind === 'image') {
      element.addEventListener(
        'load',
        () => {
          scheduleMediaHeaderLayout();
        },
        { once: true },
      );
    } else {
      element.addEventListener('loadedmetadata', () => {
        scheduleMediaHeaderLayout();
      });
    }

    applyVolume();

    if (payload.media.kind !== 'image') {
      activePlayableElement = element;
      applyMediaStartOffset(element, resolveStartOffsetSec(payload.media));

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

    const hasStandaloneMedia = !!payload?.media;
    if (!hasStandaloneMedia) {
      applyOverlayInfo(payload, {
        showAuthorInline: true,
        attachTextToMedia: false,
      });
    }
    const shouldAutoClearByTimer = !hasStandaloneMedia || payload.media.kind === 'image';

    try {
      if (!renderTweetCard(payload)) {
        await renderMedia(payload);
      }

      applyMediaHeaderAuthor(payload, {
        enabled: hasStandaloneMedia,
      });

      applyMediaFooterText(payload, {
        enabled: hasStandaloneMedia,
      });

      if (hasStandaloneMedia) {
        applyOverlayInfo(payload, {
          showAuthorInline: false,
          attachTextToMedia: true,
        });
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
      applyMediaHeaderAuthor(null, {
        enabled: false,
      });
      applyMediaFooterText(null, {
        enabled: false,
      });
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
    window.addEventListener('resize', () => {
      scheduleMediaHeaderLayout();
    });

    ensureTwitterWidgets().catch(() => undefined);

    window.livechatOverlay.rendererReady();
  };

  bootstrap().catch((error) => {
    console.error('Overlay bootstrap failed:', error);
  });
})();
