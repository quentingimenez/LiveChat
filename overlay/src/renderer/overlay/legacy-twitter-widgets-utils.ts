interface TwitterWidgetsLike {
  widgets?: {
    load?: (target?: Element) => void;
  };
  ready?: (callback: () => void) => void;
}

export interface OverlayLegacyTwitterWidgetsUtils {
  ensureTwitterWidgets(): Promise<TwitterWidgetsLike>;
}

function getTwitterGlobal(): TwitterWidgetsLike | null {
  const candidate = (window as Window & { twttr?: unknown }).twttr;
  if (typeof candidate === 'object' && candidate !== null) {
    return candidate as TwitterWidgetsLike;
  }

  return null;
}

export function createOverlayLegacyTwitterWidgetsUtils(): OverlayLegacyTwitterWidgetsUtils {
  let twitterWidgetsPromise: Promise<TwitterWidgetsLike> | null = null;

  async function ensureTwitterWidgets(): Promise<TwitterWidgetsLike> {
    const twitterGlobal = getTwitterGlobal();
    if (twitterGlobal?.widgets?.load) {
      return twitterGlobal;
    }

    if (twitterWidgetsPromise) {
      return twitterWidgetsPromise;
    }

    twitterWidgetsPromise = new Promise<TwitterWidgetsLike>((resolve, reject) => {
      const finish = (): void => {
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
