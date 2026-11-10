export function createOverlayLegacyTweetCardPlaybackUtils() {
    function renderTweetCard(params) {
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
