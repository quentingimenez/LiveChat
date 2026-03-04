interface RenderedTweetCard {
  container: HTMLElement;
  widgetTarget: HTMLElement;
  inlineVideoElements: HTMLVideoElement[];
}

interface RenderTweetCardParams {
  payload: unknown;
  createTweetCardModel: (payload: unknown) => unknown;
  createTweetCardDom: (params: { model: unknown }) => RenderedTweetCard;
  ensureMediaFrame: () => unknown;
  mediaLayer: unknown;
  scheduleMediaHeaderLayout: () => void;
  ensureTwitterWidgets: () => Promise<{ widgets: { load: (target: unknown) => void } }>;
  configureInlineVideoLooping: (videos: HTMLVideoElement[]) => void;
  playVideoWithRetry: (video: HTMLVideoElement) => Promise<void>;
  applyVolume: () => void;
  ensureInlineVideoAudioFallback: (videos: HTMLVideoElement[]) => Promise<void>;
  setTimer: (callback: () => void, delayMs: number) => number;
  logWarning: (message: string, error: unknown) => void;
}

export interface OverlayLegacyTweetCardPlaybackUtils {
  renderTweetCard(params: RenderTweetCardParams): boolean;
}

export function createOverlayLegacyTweetCardPlaybackUtils(): OverlayLegacyTweetCardPlaybackUtils {
  function renderTweetCard(params: RenderTweetCardParams): boolean {
    const tweetCardModel = params.createTweetCardModel(params.payload);
    if (!tweetCardModel) {
      return false;
    }

    const renderedTweetCard = params.createTweetCardDom({
      model: tweetCardModel,
    });

    const targetContainer = params.ensureMediaFrame() || params.mediaLayer;
    if (!(targetContainer instanceof HTMLElement)) {
      return false;
    }

    targetContainer.appendChild(renderedTweetCard.container);
    params.scheduleMediaHeaderLayout();

    params
      .ensureTwitterWidgets()
      .then((twitter) => {
        twitter.widgets.load(renderedTweetCard.widgetTarget);
      })
      .catch((error) => {
        params.logWarning('Twitter widgets fallback to raw HTML:', error);
      });

    if (renderedTweetCard.inlineVideoElements.length > 0) {
      params.configureInlineVideoLooping(renderedTweetCard.inlineVideoElements);
      renderedTweetCard.inlineVideoElements.forEach((video) => {
        video.addEventListener('loadedmetadata', () => {
          params.scheduleMediaHeaderLayout();
        });
      });

      renderedTweetCard.inlineVideoElements.forEach((video) => {
        void params.playVideoWithRetry(video);
      });

      params.setTimer(() => {
        params.applyVolume();
      }, 220);

      void params.ensureInlineVideoAudioFallback(renderedTweetCard.inlineVideoElements);
    }

    return true;
  }

  return {
    renderTweetCard,
  };
}
