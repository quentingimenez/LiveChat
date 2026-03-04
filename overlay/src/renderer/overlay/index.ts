import { loadLegacyRendererScript } from '../legacy-loader.js';
import { createOverlayLegacyInlineVideoUtils } from './legacy-inline-video-utils.js';
import { createOverlayLegacyMediaDiagnosticsUtils } from './legacy-media-diagnostics.js';
import { createOverlayLegacyMediaDomUtils } from './legacy-media-dom-utils.js';
import { createOverlayLegacyMediaFrameUtils } from './legacy-media-frame-utils.js';
import { createOverlayLegacyMediaFrameLayoutUtils } from './legacy-media-frame-layout-utils.js';
import { createOverlayLegacyMediaRenderUtils } from './legacy-media-render-utils.js';
import { createOverlayLegacyMediaOffsetUtils } from './legacy-media-offset-utils.js';
import { createOverlayLegacyPlayFlowUtils } from './legacy-play-flow-utils.js';
import { createOverlayLegacyCountdownUtils } from './legacy-countdown-utils.js';
import { createOverlayLegacyOverlayResetUtils } from './legacy-overlay-reset-utils.js';
import { createOverlayLegacyPlaybackUtils } from './legacy-playback-utils.js';
import { createOverlayLegacyPlaybackSessionUtils } from './legacy-playback-session-utils.js';
import { createOverlayLegacyBootstrapUtils } from './legacy-bootstrap-utils.js';
import { createOverlayLegacyTextUtils } from './legacy-overlay-text-utils.js';
import { createOverlayLegacyTweetCardRendererUtils } from './legacy-tweet-card-renderer.js';
import { createOverlayLegacyTweetCardPlaybackUtils } from './legacy-tweet-card-playback-utils.js';
import { createOverlayLegacyTweetCardUtils } from './legacy-tweet-card-utils.js';
import { createOverlayLegacyTwitterWidgetsUtils } from './legacy-twitter-widgets-utils.js';
import { createOverlayLegacyRuntimeInputsUtils } from './legacy-runtime-inputs-utils.js';
import { createOverlayLegacyRuntimeMediaFrameHandlersUtils } from './legacy-runtime-media-frame-handlers-utils.js';
import { createOverlayLegacyRuntimePlaybackCountdownHandlersUtils } from './legacy-runtime-playback-countdown-handlers-utils.js';
import { createOverlayLegacyRuntimeRenderHandlersUtils } from './legacy-runtime-render-handlers-utils.js';
import { createOverlayLegacyUtils } from './legacy-utils.js';

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
