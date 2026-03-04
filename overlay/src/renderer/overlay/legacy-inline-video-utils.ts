export interface PlayVideoWithRetryOptions {
  attempts?: number;
  delayMs?: number;
}

export interface OverlayLegacyInlineVideoUtils {
  videoHasAudioTrack(video: unknown): boolean;
  playVideoWithRetry(video: unknown, options?: PlayVideoWithRetryOptions): void;
  configureInlineVideoLooping(videos: unknown): void;
  ensureInlineVideoAudioFallback(videos: unknown, applyVolume: () => void): void;
}

interface CreateOverlayLegacyInlineVideoUtilsOptions {
  getDurationSec(value: unknown): number | null;
  logWarning(message: string, error?: unknown): void;
}

type VideoAudioTrackLike = HTMLVideoElement & {
  mozHasAudio?: boolean;
  webkitAudioDecodedByteCount?: number;
  audioTracks?: {
    length?: number;
  };
};

function toVideoList(value: unknown): HTMLVideoElement[] {
  if (!Array.isArray(value)) {
    return [];
  }

  return value.filter((entry): entry is HTMLVideoElement => entry instanceof HTMLVideoElement);
}

export function createOverlayLegacyInlineVideoUtils(
  options: CreateOverlayLegacyInlineVideoUtilsOptions
): OverlayLegacyInlineVideoUtils {
  const { getDurationSec, logWarning } = options;

  function videoHasAudioTrack(video: unknown): boolean {
    if (!(video instanceof HTMLVideoElement)) {
      return false;
    }

    const candidate = video as VideoAudioTrackLike;

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
  }

  function playVideoWithRetry(video: unknown, retryOptions: PlayVideoWithRetryOptions = {}): void {
    if (!(video instanceof HTMLVideoElement)) {
      return;
    }

    const attempts =
      typeof retryOptions.attempts === 'number' && Number.isFinite(retryOptions.attempts)
        ? Math.max(0, Math.floor(retryOptions.attempts))
        : 4;
    const delayMs =
      typeof retryOptions.delayMs === 'number' && Number.isFinite(retryOptions.delayMs)
        ? Math.max(0, Math.floor(retryOptions.delayMs))
        : 250;

    const attemptPlay = (remainingAttempts: number): void => {
      video.play().catch((error) => {
        if (remainingAttempts <= 0) {
          logWarning('Tweet inline video autoplay failed:', error);
          return;
        }

        setTimeout(() => attemptPlay(remainingAttempts - 1), delayMs);
      });
    };

    attemptPlay(attempts);
  }

  function configureInlineVideoLooping(videos: unknown): void {
    const videoList = toVideoList(videos);
    if (videoList.length < 2) {
      return;
    }

    const configuredDurations = videoList.map((video) => {
      const raw = Number.parseFloat(video.dataset.configDurationSec || '');
      return getDurationSec(raw);
    });

    const applyLoopPolicy = (): void => {
      const measuredDurations = videoList.map((video, index) => {
        const measured = getDurationSec(video.duration);
        return measured ?? configuredDurations[index] ?? null;
      });

      const knownDurations = measuredDurations.filter((durationSec): durationSec is number => typeof durationSec === 'number');
      let longestIndex = 0;

      if (knownDurations.length > 0) {
        const longestDuration = Math.max(...knownDurations);
        longestIndex = measuredDurations.findIndex((durationSec) => durationSec === longestDuration);
        if (longestIndex < 0) {
          longestIndex = 0;
        }
      }

      videoList.forEach((video, index) => {
        const durationSec = measuredDurations[index];
        const longestDurationCandidate = measuredDurations[longestIndex];
        const longestDuration = typeof longestDurationCandidate === 'number' ? longestDurationCandidate : null;
        const shouldLoop =
          knownDurations.length === 0
            ? index > 0
            : index !== longestIndex &&
              (durationSec === null || (typeof longestDuration === 'number' && durationSec + 0.25 < longestDuration));

        video.loop = shouldLoop;
        video.dataset.loopWanted = shouldLoop ? '1' : '0';
      });
    };

    videoList.forEach((video) => {
      video.addEventListener('loadedmetadata', applyLoopPolicy);
      video.addEventListener('ended', () => {
        if (video.dataset.loopWanted !== '1') {
          return;
        }

        video.currentTime = 0;
        playVideoWithRetry(video, {
          attempts: 2,
          delayMs: 120
        });
      });
    });

    applyLoopPolicy();
  }

  function ensureInlineVideoAudioFallback(videos: unknown, applyVolume: () => void): void {
    const videoList = toVideoList(videos);
    if (videoList.length < 2) {
      return;
    }

    const primary = videoList[0];
    if (!(primary instanceof HTMLVideoElement)) {
      return;
    }

    const tryPromoteAudibleVideo = (): void => {
      if (videoHasAudioTrack(primary)) {
        return;
      }

      const replacement = videoList.slice(1).find((video) => videoHasAudioTrack(video));
      if (!replacement) {
        return;
      }

      primary.dataset.forceMuted = '1';
      replacement.dataset.forceMuted = '0';
      applyVolume();
    };

    setTimeout(tryPromoteAudibleVideo, 1000);
    setTimeout(tryPromoteAudibleVideo, 2200);
  }

  return {
    videoHasAudioTrack,
    playVideoWithRetry,
    configureInlineVideoLooping,
    ensureInlineVideoAudioFallback
  };
}
