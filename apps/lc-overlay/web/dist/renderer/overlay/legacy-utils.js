export function createOverlayLegacyUtils() {
    function formatRemainingTime(remainingMs) {
        const remainingSec = Math.max(0, Math.ceil(remainingMs / 1000));
        const minutes = Math.floor(remainingSec / 60);
        const seconds = remainingSec % 60;
        return `${minutes.toString().padStart(2, '0')}:${seconds.toString().padStart(2, '0')}`;
    }
    function getDurationSec(value) {
        if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
            return null;
        }
        return value;
    }
    function getStartOffsetSec(value) {
        if (typeof value !== 'number' || !Number.isFinite(value)) {
            return null;
        }
        const normalized = Math.floor(value);
        return normalized > 0 ? normalized : null;
    }
    function parseStartOffsetFromMediaUrl(rawUrl) {
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
        }
        catch {
            return null;
        }
    }
    function resolveStartOffsetSec(mediaPayload) {
        const payload = typeof mediaPayload === 'object' && mediaPayload !== null ? mediaPayload : {};
        const directOffset = getStartOffsetSec(payload.startOffsetSec);
        if (directOffset !== null) {
            return directOffset;
        }
        return parseStartOffsetFromMediaUrl(payload.url);
    }
    function clamp01(value) {
        if (typeof value !== 'number' || !Number.isFinite(value)) {
            return 1;
        }
        return Math.min(1, Math.max(0, value));
    }
    function toPerceptualGain(linearVolume, gamma = 2.2) {
        const normalized = clamp01(linearVolume);
        return Math.pow(normalized, gamma);
    }
    function toRedactedMediaUrl(value) {
        if (typeof value !== 'string' || value.trim() === '') {
            return 'unknown-url';
        }
        try {
            const parsed = new URL(value);
            if (parsed.searchParams.has('token')) {
                parsed.searchParams.set('token', '***');
            }
            return parsed.toString();
        }
        catch {
            return value;
        }
    }
    function hasTokenInMediaUrl(value) {
        if (typeof value !== 'string' || value.trim() === '') {
            return false;
        }
        try {
            const parsed = new URL(value);
            const token = parsed.searchParams.get('token');
            return typeof token === 'string' && token.trim() !== '';
        }
        catch {
            return false;
        }
    }
    function getHtmlMediaErrorLabel(code) {
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
    }
    return {
        formatRemainingTime,
        getDurationSec,
        getStartOffsetSec,
        parseStartOffsetFromMediaUrl,
        resolveStartOffsetSec,
        clamp01,
        toPerceptualGain,
        toRedactedMediaUrl,
        hasTokenInMediaUrl,
        getHtmlMediaErrorLabel
    };
}
