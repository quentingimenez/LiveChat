import type {
  OverlayTweetCardModel,
  OverlayTweetInlineRoleLabelInput
} from './legacy-tweet-card-utils.js';

interface CreateTweetCardDomParams {
  model: OverlayTweetCardModel;
}

interface CreateOverlayLegacyTweetCardRendererUtilsOptions {
  getInlineVideoRoleLabel(input: OverlayTweetInlineRoleLabelInput): string;
}

export interface OverlayTweetCardDomResult {
  container: HTMLElement;
  widgetTarget: HTMLElement;
  inlineVideoElements: HTMLVideoElement[];
}

export interface OverlayLegacyTweetCardRendererUtils {
  createTweetCardDom(params: CreateTweetCardDomParams): OverlayTweetCardDomResult;
}

function applyExternalLinkAttributes(root: Element): void {
  const links = root.querySelectorAll('a[href]');
  links.forEach((link) => {
    link.setAttribute('target', '_blank');
    link.setAttribute('rel', 'noopener noreferrer');
  });
}

export function createOverlayLegacyTweetCardRendererUtils(
  options: CreateOverlayLegacyTweetCardRendererUtilsOptions
): OverlayLegacyTweetCardRendererUtils {
  function createInlineVideoNode(
    model: OverlayTweetCardModel,
    index: number
  ): {
    inlineItemNode: HTMLElement;
    videoNode: HTMLVideoElement;
  } {
    const inlineVideo = model.videos[index];
    const inlineItemNode = document.createElement('div');
    inlineItemNode.className = 'overlay-tweet-inline-item';

    const roleLabelNode = document.createElement('div');
    roleLabelNode.className = 'overlay-tweet-inline-label';
    roleLabelNode.textContent = options.getInlineVideoRoleLabel({
      index,
      total: model.videos.length,
      hasReplyContext: model.hasReplyContext,
      currentStatusId: model.currentStatusId,
      sourceStatusId: inlineVideo.sourceStatusId
    });
    inlineItemNode.appendChild(roleLabelNode);

    const videoNode = document.createElement('video');
    videoNode.className = 'overlay-tweet-inline-video';
    videoNode.autoplay = true;
    videoNode.controls = false;
    videoNode.playsInline = true;
    videoNode.preload = 'auto';
    videoNode.dataset.configDurationSec =
      typeof inlineVideo.durationSec === 'number' && Number.isFinite(inlineVideo.durationSec) && inlineVideo.durationSec > 0
        ? `${inlineVideo.durationSec}`
        : '';
    videoNode.muted = true;
    videoNode.dataset.forceMuted = index > 0 ? '1' : '0';
    videoNode.src = inlineVideo.url;

    if (typeof inlineVideo.isVertical === 'boolean') {
      videoNode.dataset.vertical = inlineVideo.isVertical ? '1' : '0';
    }

    inlineItemNode.appendChild(videoNode);

    return {
      inlineItemNode,
      videoNode
    };
  }

  function createTweetCardDom(params: CreateTweetCardDomParams): OverlayTweetCardDomResult {
    const { model } = params;

    if (model.videos.length > 0) {
      const container = document.createElement('div');
      container.className = 'overlay-tweet-card overlay-tweet-card-with-video';
      if (model.videos.length > 1) {
        container.classList.add('overlay-tweet-card-with-multi-video');
      }

      const widgetContent = document.createElement('div');
      widgetContent.className = 'overlay-tweet-card-content overlay-tweet-widget';
      widgetContent.innerHTML = model.html;
      applyExternalLinkAttributes(widgetContent);
      container.appendChild(widgetContent);

      const inlineMediaNode = document.createElement('div');
      inlineMediaNode.className = 'overlay-tweet-inline-media';
      inlineMediaNode.dataset.count = `${model.videos.length}`;

      const inlineVideoElements: HTMLVideoElement[] = [];
      for (let index = 0; index < model.videos.length; index += 1) {
        const { inlineItemNode, videoNode } = createInlineVideoNode(model, index);
        inlineMediaNode.appendChild(inlineItemNode);
        inlineVideoElements.push(videoNode);
      }

      container.appendChild(inlineMediaNode);

      return {
        container,
        widgetTarget: widgetContent,
        inlineVideoElements
      };
    }

    const container = document.createElement('div');
    container.className = 'overlay-tweet-card';

    const widgetContent = document.createElement('div');
    widgetContent.className = 'overlay-tweet-card-content';
    widgetContent.innerHTML = model.html;
    applyExternalLinkAttributes(widgetContent);
    container.appendChild(widgetContent);

    return {
      container,
      widgetTarget: widgetContent,
      inlineVideoElements: []
    };
  }

  return {
    createTweetCardDom
  };
}
