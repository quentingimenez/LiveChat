export interface OverlayAuthorInfo {
  showAuthor: boolean;
  authorName: string;
  authorImage: string | null;
}

export interface ApplyMediaHeaderAuthorParams {
  container: unknown;
  payload: unknown;
  showText: boolean;
  enabled: boolean;
  onLayout?: () => void;
}

export interface ApplyMediaFooterTextParams {
  container: unknown;
  payload: unknown;
  showText: boolean;
  enabled: boolean;
}

export interface ApplyOverlayInfoParams {
  layer: unknown;
  payload: unknown;
  showText: boolean;
  showAuthorInline: boolean;
  attachTextToMedia: boolean;
}

export interface OverlayLegacyTextUtils {
  resolveAuthorInfo(payload: unknown): OverlayAuthorInfo;
  applyMediaHeaderAuthor(params: ApplyMediaHeaderAuthorParams): void;
  applyMediaFooterText(params: ApplyMediaFooterTextParams): void;
  applyOverlayInfo(params: ApplyOverlayInfoParams): void;
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function toTrimmedString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : '';
}

function resolveOverlayText(payload: unknown): {
  isEnabled: boolean;
  value: string;
  show: boolean;
} {
  const payloadRecord = toRecord(payload);
  const textRecord = toRecord(payloadRecord.text);
  const textEnabled = textRecord.enabled === true;
  const textValue = toTrimmedString(textRecord.value);

  return {
    isEnabled: textEnabled,
    value: textValue,
    show: textEnabled && textValue !== ''
  };
}

export function createOverlayLegacyTextUtils(): OverlayLegacyTextUtils {
  function resolveAuthorInfo(payload: unknown): OverlayAuthorInfo {
    const payloadRecord = toRecord(payload);
    const authorRecord = toRecord(payloadRecord.author);
    const authorEnabled = authorRecord.enabled === true;
    const authorName = toTrimmedString(authorRecord.name);
    const authorImage = toTrimmedString(authorRecord.image) || null;
    const showAuthor = authorEnabled && authorName !== '';

    return {
      showAuthor,
      authorName,
      authorImage
    };
  }

  function createAvatarNode(authorName: string, authorImage: string | null): HTMLElement {
    if (authorImage) {
      const avatarImage = document.createElement('img');
      avatarImage.className = 'overlay-author-avatar';
      avatarImage.src = authorImage;
      avatarImage.alt = authorName;
      avatarImage.referrerPolicy = 'no-referrer';
      avatarImage.addEventListener('error', () => {
        avatarImage.replaceWith(createAvatarNode(authorName, null));
      });
      return avatarImage;
    }

    const fallback = document.createElement('div');
    fallback.className = 'overlay-author-avatar-fallback';
    fallback.textContent = authorName.trim().charAt(0).toUpperCase() || '?';
    return fallback;
  }

  function createAuthorNode(authorName: string, authorImage: string | null): HTMLElement {
    const authorNode = document.createElement('div');
    authorNode.className = 'overlay-author';
    authorNode.appendChild(createAvatarNode(authorName, authorImage));

    const authorNameNode = document.createElement('div');
    authorNameNode.className = 'overlay-author-name';
    authorNameNode.textContent = authorName;
    authorNode.appendChild(authorNameNode);

    return authorNode;
  }

  function applyMediaHeaderAuthor(params: ApplyMediaHeaderAuthorParams): void {
    const container = params.container;
    if (!(container instanceof HTMLElement)) {
      return;
    }

    container.innerHTML = '';

    if (!params.showText || params.enabled !== true) {
      params.onLayout?.();
      return;
    }

    const authorInfo = resolveAuthorInfo(params.payload);
    if (!authorInfo.showAuthor) {
      params.onLayout?.();
      return;
    }

    container.appendChild(createAuthorNode(authorInfo.authorName, authorInfo.authorImage));
    params.onLayout?.();
  }

  function applyMediaFooterText(params: ApplyMediaFooterTextParams): void {
    const container = params.container;
    if (!(container instanceof HTMLElement)) {
      return;
    }

    container.innerHTML = '';

    if (!params.showText || params.enabled !== true) {
      container.style.display = 'none';
      return;
    }

    const overlayText = resolveOverlayText(params.payload);
    if (!overlayText.show) {
      container.style.display = 'none';
      return;
    }

    const metaNode = document.createElement('div');
    metaNode.className = 'overlay-meta overlay-media-footer-meta';

    const textNode = document.createElement('div');
    textNode.className = 'overlay-text-value';
    textNode.textContent = overlayText.value;
    metaNode.appendChild(textNode);

    container.appendChild(metaNode);
    container.style.display = 'flex';
  }

  function applyOverlayInfo(params: ApplyOverlayInfoParams): void {
    const layer = params.layer;
    if (!(layer instanceof HTMLElement)) {
      return;
    }

    if (!params.showText) {
      layer.innerHTML = '';
      layer.style.display = 'none';
      return;
    }

    const authorInfo = resolveAuthorInfo(params.payload);
    const overlayText = resolveOverlayText(params.payload);
    const showTextInOverlayLayer = overlayText.show && !params.attachTextToMedia;

    if (!(params.showAuthorInline && authorInfo.showAuthor) && !showTextInOverlayLayer) {
      layer.innerHTML = '';
      layer.style.display = 'none';
      return;
    }

    layer.innerHTML = '';

    const metaNode = document.createElement('div');
    metaNode.className = 'overlay-meta';

    if (params.showAuthorInline && authorInfo.showAuthor) {
      metaNode.appendChild(createAuthorNode(authorInfo.authorName, authorInfo.authorImage));
    }

    if (showTextInOverlayLayer) {
      const textNode = document.createElement('div');
      textNode.className = 'overlay-text-value';
      textNode.textContent = overlayText.value;
      metaNode.appendChild(textNode);
    }

    layer.appendChild(metaNode);
    layer.style.display = 'block';
  }

  return {
    resolveAuthorInfo,
    applyMediaHeaderAuthor,
    applyMediaFooterText,
    applyOverlayInfo
  };
}
