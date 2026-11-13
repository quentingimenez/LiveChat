function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function toNonEmptyString(value) {
    if (typeof value !== 'string') {
        return '';
    }
    const normalized = value.trim();
    return normalized ? normalized : '';
}
function toNullableString(value) {
    const normalized = toNonEmptyString(value);
    return normalized ? normalized : null;
}
function toNullableBoolean(value) {
    return typeof value === 'boolean' ? value : null;
}
function toNullableDurationSec(value) {
    return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : null;
}
function toInlineVideoCandidate(value) {
    const record = toRecord(value);
    const url = toNonEmptyString(record.url);
    if (!url) {
        return null;
    }
    return {
        url,
        isVertical: toNullableBoolean(record.isVertical),
        sourceStatusId: toNullableString(record.sourceStatusId),
        durationSec: toNullableDurationSec(record.durationSec)
    };
}
function buildInlineVideoKey(video) {
    const sourceStatusId = video.sourceStatusId || 'source-unknown';
    return `${sourceStatusId}:${video.url}`;
}
export function createOverlayLegacyTweetCardUtils() {
    function createTweetCardModel(payload) {
        const payloadRecord = toRecord(payload);
        const tweetCard = toRecord(payloadRecord.tweetCard);
        const html = toNonEmptyString(tweetCard.html);
        if (!html) {
            return null;
        }
        const inlineVideos = [];
        const seenInlineVideoKeys = new Set();
        const pushInlineVideo = (candidateValue) => {
            const candidate = toInlineVideoCandidate(candidateValue);
            if (!candidate) {
                return;
            }
            const key = buildInlineVideoKey(candidate);
            if (seenInlineVideoKeys.has(key)) {
                return;
            }
            seenInlineVideoKeys.add(key);
            inlineVideos.push(candidate);
        };
        const videos = tweetCard.videos;
        if (Array.isArray(videos)) {
            videos.forEach((video) => {
                pushInlineVideo(video);
            });
        }
        if (inlineVideos.length === 0) {
            const fallbackVideoUrl = toNonEmptyString(tweetCard.videoUrl);
            if (fallbackVideoUrl) {
                pushInlineVideo({
                    url: fallbackVideoUrl,
                    isVertical: toNullableBoolean(tweetCard.videoIsVertical),
                    sourceStatusId: null,
                    durationSec: null
                });
            }
        }
        const videosToRender = inlineVideos.slice(0, 2);
        const currentStatusId = toNullableString(tweetCard.currentStatusId);
        const hasCurrentTweetVideo = !!currentStatusId &&
            videosToRender.some((video) => typeof video.sourceStatusId === 'string' && video.sourceStatusId === currentStatusId);
        const hasOriginalTweetVideo = !!currentStatusId &&
            videosToRender.some((video) => typeof video.sourceStatusId === 'string' &&
                video.sourceStatusId.trim() !== '' &&
                video.sourceStatusId !== currentStatusId);
        return {
            html,
            videos: videosToRender,
            currentStatusId,
            hasReplyContext: hasCurrentTweetVideo && hasOriginalTweetVideo
        };
    }
    function getInlineVideoRoleLabel(input) {
        const index = Number.isFinite(input.index) ? Math.max(0, Math.floor(input.index)) : 0;
        const total = Number.isFinite(input.total) ? Math.max(0, Math.floor(input.total)) : 0;
        if (input.hasReplyContext && input.currentStatusId && input.sourceStatusId) {
            return input.sourceStatusId === input.currentStatusId ? 'Reponse' : 'Tweet original';
        }
        if (total <= 1) {
            return 'Video';
        }
        return index === 0 ? 'Video 1' : 'Video 2';
    }
    return {
        createTweetCardModel,
        getInlineVideoRoleLabel
    };
}
