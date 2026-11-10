function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function toFiniteNumber(value, fallback) {
    return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}
function clamp01(value) {
    const numeric = toFiniteNumber(value, 1);
    if (numeric <= 0) {
        return 0;
    }
    if (numeric >= 1) {
        return 1;
    }
    return numeric;
}
function toPerceptualGain(linearVolume, gamma) {
    return Math.pow(clamp01(linearVolume), toFiniteNumber(gamma, 2.2));
}
export function createBoardLegacyPreviewMediaUtils() {
    function buildAuthedUrl(params) {
        const config = toRecord(params.config);
        const baseUrl = `${config.serverUrl || ''}`.trim();
        const token = `${config.clientToken || ''}`.trim();
        const url = new URL(params.pathname, baseUrl);
        if (token) {
            url.searchParams.set('token', token);
        }
        const options = params.options || {};
        for (const [key, value] of Object.entries(options)) {
            if (value === null || value === undefined || value === '') {
                continue;
            }
            url.searchParams.set(key, `${value}`);
        }
        return url;
    }
    function applyPreviewVolume(params) {
        if (!(params.root instanceof Element)) {
            return;
        }
        const gain = toPerceptualGain(params.linearVolume, params.gamma);
        const mediaElements = params.root.querySelectorAll('audio,video');
        mediaElements.forEach((element) => {
            if (element instanceof HTMLMediaElement) {
                element.volume = gain;
                element.muted = false;
            }
        });
    }
    return {
        buildAuthedUrl,
        applyPreviewVolume,
    };
}
