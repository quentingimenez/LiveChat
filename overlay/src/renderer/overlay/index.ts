import { loadLegacyRendererScript } from '../legacy-loader';
import { createOverlayLegacyInlineVideoUtils } from './legacy-inline-video-utils';
import { createOverlayLegacyMediaDiagnosticsUtils } from './legacy-media-diagnostics';
import { createOverlayLegacyMediaDomUtils } from './legacy-media-dom-utils';
import { createOverlayLegacyMediaFrameUtils } from './legacy-media-frame-utils';
import { createOverlayLegacyMediaFrameLayoutUtils } from './legacy-media-frame-layout-utils';
import { createOverlayLegacyMediaRenderUtils } from './legacy-media-render-utils';
import { createOverlayLegacyMediaOffsetUtils } from './legacy-media-offset-utils';
import { createOverlayLegacyPlayFlowUtils } from './legacy-play-flow-utils';
import { createOverlayLegacyCountdownUtils } from './legacy-countdown-utils';
import { createOverlayLegacyOverlayResetUtils } from './legacy-overlay-reset-utils';
import { createOverlayLegacyPlaybackUtils } from './legacy-playback-utils';
import { createOverlayLegacyPlaybackSessionUtils } from './legacy-playback-session-utils';
import { createOverlayLegacyBootstrapUtils } from './legacy-bootstrap-utils';
import { createOverlayLegacyTextUtils } from './legacy-overlay-text-utils';
import { createOverlayLegacyTweetCardRendererUtils } from './legacy-tweet-card-renderer';
import { createOverlayLegacyTweetCardPlaybackUtils } from './legacy-tweet-card-playback-utils';
import { createOverlayLegacyTweetCardUtils } from './legacy-tweet-card-utils';
import { createOverlayLegacyTwitterWidgetsUtils } from './legacy-twitter-widgets-utils';
import { createOverlayLegacyRuntimeInputsUtils } from './legacy-runtime-inputs-utils';
import { createOverlayLegacyRuntimeMediaFrameHandlersUtils } from './legacy-runtime-media-frame-handlers-utils';
import { createOverlayLegacyRuntimePlaybackCountdownHandlersUtils } from './legacy-runtime-playback-countdown-handlers-utils';
import { createOverlayLegacyRuntimeRenderHandlersUtils } from './legacy-runtime-render-handlers-utils';
import { createOverlayLegacyUtils } from './legacy-utils';

const overlayLegacyUtils = createOverlayLegacyUtils();
window.__overlayLegacyUtils = overlayLegacyUtils;
window.__overlayLegacyMediaDomUtils = createOverlayLegacyMediaDomUtils();
window.__overlayLegacyMediaFrameUtils = createOverlayLegacyMediaFrameUtils();
window.__overlayLegacyMediaFrameLayoutUtils = createOverlayLegacyMediaFrameLayoutUtils();
window.__overlayLegacyMediaRenderUtils = createOverlayLegacyMediaRenderUtils();
window.__overlayLegacyPlayFlowUtils = createOverlayLegacyPlayFlowUtils();
window.__overlayLegacyCountdownUtils = createOverlayLegacyCountdownUtils();
window.__overlayLegacyPlaybackUtils = createOverlayLegacyPlaybackUtils();
window.__overlayLegacyPlaybackSessionUtils = createOverlayLegacyPlaybackSessionUtils();
window.__overlayLegacyBootstrapUtils = createOverlayLegacyBootstrapUtils();
window.__overlayLegacyOverlayResetUtils = createOverlayLegacyOverlayResetUtils();
window.__overlayLegacyDiagnosticsUtils = createOverlayLegacyMediaDiagnosticsUtils({
  toRedactedMediaUrl: overlayLegacyUtils.toRedactedMediaUrl,
  hasTokenInMediaUrl: overlayLegacyUtils.hasTokenInMediaUrl,
  getHtmlMediaErrorLabel: overlayLegacyUtils.getHtmlMediaErrorLabel,
  logWarning: (message) => {
    console.warn(message);
  }
});
window.__overlayLegacyMediaOffsetUtils = createOverlayLegacyMediaOffsetUtils({
  getStartOffsetSec: overlayLegacyUtils.getStartOffsetSec,
  logWarning: (message, error) => {
    console.warn(message, error);
  }
});
window.__overlayLegacyInlineVideoUtils = createOverlayLegacyInlineVideoUtils({
  getDurationSec: overlayLegacyUtils.getDurationSec,
  logWarning: (message, error) => {
    console.warn(message, error);
  }
});
window.__overlayLegacyTextUtils = createOverlayLegacyTextUtils();
window.__overlayLegacyTwitterWidgetsUtils = createOverlayLegacyTwitterWidgetsUtils();
const overlayLegacyTweetCardUtils = createOverlayLegacyTweetCardUtils();
window.__overlayLegacyTweetCardUtils = overlayLegacyTweetCardUtils;
window.__overlayLegacyTweetCardPlaybackUtils = createOverlayLegacyTweetCardPlaybackUtils();
window.__overlayLegacyTweetCardRendererUtils = createOverlayLegacyTweetCardRendererUtils({
  getInlineVideoRoleLabel: overlayLegacyTweetCardUtils.getInlineVideoRoleLabel
});
window.__overlayLegacyRuntimeInputsUtils = createOverlayLegacyRuntimeInputsUtils();
window.__overlayLegacyRuntimeMediaFrameHandlersUtils = createOverlayLegacyRuntimeMediaFrameHandlersUtils();
window.__overlayLegacyRuntimePlaybackCountdownHandlersUtils = createOverlayLegacyRuntimePlaybackCountdownHandlersUtils();
window.__overlayLegacyRuntimeRenderHandlersUtils = createOverlayLegacyRuntimeRenderHandlersUtils();

loadLegacyRendererScript('./overlay.js');
