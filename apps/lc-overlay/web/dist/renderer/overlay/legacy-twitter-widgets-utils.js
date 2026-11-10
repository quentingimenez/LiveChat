function getTwitterGlobal() {
    const candidate = window.twttr;
    if (typeof candidate === 'object' && candidate !== null) {
        return candidate;
    }
    return null;
}
export function createOverlayLegacyTwitterWidgetsUtils() {
    let twitterWidgetsPromise = null;
    async function ensureTwitterWidgets() {
        const twitterGlobal = getTwitterGlobal();
        if (twitterGlobal?.widgets?.load) {
            return twitterGlobal;
        }
        if (twitterWidgetsPromise) {
            return twitterWidgetsPromise;
        }
        twitterWidgetsPromise = new Promise((resolve, reject) => {
            const finish = () => {
                const loadedTwitterGlobal = getTwitterGlobal();
                if (loadedTwitterGlobal?.widgets?.load) {
                    resolve(loadedTwitterGlobal);
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
                const loadedTwitterGlobal = getTwitterGlobal();
                if (loadedTwitterGlobal?.ready) {
                    loadedTwitterGlobal.ready(() => finish());
                    return;
                }
                setTimeout(finish, 0);
            };
            script.onerror = () => {
                reject(new Error('twitter_widgets_load_failed'));
            };
            document.head.appendChild(script);
        }).catch((error) => {
            twitterWidgetsPromise = null;
            throw error;
        });
        return twitterWidgetsPromise;
    }
    return {
        ensureTwitterWidgets
    };
}
