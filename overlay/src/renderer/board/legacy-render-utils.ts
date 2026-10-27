type MediaKind = 'image' | 'audio' | 'video';

interface CreateEmptyStateNodeParams {
  text: string;
}

interface CreatePreviewMediaNodeParams {
  mediaKind: MediaKind;
  mediaUrl: string;
  title: string;
  onMediaReady: () => void;
}

interface CreateItemCardParams {
  item: unknown;
  isSelected: boolean;
  shortcuts: string[];
  toCardTitle: (item: unknown) => string;
  toSafeDateLabel: (value: unknown) => string;
  toMessagePreview: (value: unknown) => string;
  onSelect: () => void;
  onTrigger: () => void;
  onRename: () => void;
  onBind: () => void;
  onDelete: () => void;
}

interface BuildPreviewMetaTextParams {
  mediaKind: unknown;
  hasMessage: boolean;
  shortcuts: string[];
}

export interface BoardLegacyRenderUtils {
  createEmptyStateNode(params: CreateEmptyStateNodeParams): HTMLElement;
  createPreviewMediaNode(params: CreatePreviewMediaNodeParams): HTMLImageElement | HTMLAudioElement | HTMLVideoElement;
  createItemCard(params: CreateItemCardParams): HTMLElement;
  buildPreviewMetaText(params: BuildPreviewMetaTextParams): string;
}

function toRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
}

function toTrimmedString(value: unknown): string {
  return `${value || ''}`.trim();
}

function toTextLabel(value: unknown, fallback: string): string {
  const normalized = toTrimmedString(value);
  return normalized || fallback;
}

function toMediaKindLabel(value: unknown): string {
  const normalized = toTrimmedString(value);
  return normalized || 'MEDIA';
}

export function createBoardLegacyRenderUtils(): BoardLegacyRenderUtils {
  function createEmptyStateNode(params: CreateEmptyStateNodeParams): HTMLElement {
    const emptyNode = document.createElement('p');
    emptyNode.className = 'preview-empty';
    emptyNode.textContent = params.text;
    return emptyNode;
  }

  function createPreviewMediaNode(
    params: CreatePreviewMediaNodeParams
  ): HTMLImageElement | HTMLAudioElement | HTMLVideoElement {
    if (params.mediaKind === 'image') {
      const mediaNode = document.createElement('img');
      mediaNode.src = params.mediaUrl;
      mediaNode.alt = params.title;
      mediaNode.loading = 'lazy';
      mediaNode.addEventListener('load', () => {
        params.onMediaReady();
      });
      mediaNode.className = 'preview-media';
      return mediaNode;
    }

    if (params.mediaKind === 'audio') {
      const mediaNode = document.createElement('audio');
      mediaNode.src = params.mediaUrl;
      mediaNode.controls = true;
      mediaNode.preload = 'metadata';
      mediaNode.className = 'preview-media';
      return mediaNode;
    }

    const mediaNode = document.createElement('video');
    mediaNode.src = params.mediaUrl;
    mediaNode.controls = true;
    mediaNode.preload = 'metadata';
    mediaNode.addEventListener('loadedmetadata', () => {
      params.onMediaReady();
    });
    mediaNode.className = 'preview-media';
    return mediaNode;
  }

  function createItemCard(params: CreateItemCardParams): HTMLElement {
    const itemRecord = toRecord(params.item);
    const itemId = toTrimmedString(itemRecord.id);
    const mediaRecord = toRecord(itemRecord.media);
    const createdByName = toTextLabel(itemRecord.createdByName, 'Auteur inconnu');
    const messagePreview = params.toMessagePreview(itemRecord.message);

    const card = document.createElement('article');
    card.className = `item-card${params.isSelected ? ' selected' : ''}`;
    card.addEventListener('click', (event) => {
      const target = event.target;
      if (target instanceof Element && target.closest('button')) {
        return;
      }

      params.onSelect();
    });

    const titleNode = document.createElement('h3');
    titleNode.className = 'item-title';
    titleNode.textContent = params.toCardTitle(itemRecord);

    const subNode = document.createElement('p');
    subNode.className = 'item-subline';
    subNode.textContent = `${toMediaKindLabel(mediaRecord.kind)} | ${params.toSafeDateLabel(itemRecord.createdAt)}`;

    const authorNode = document.createElement('p');
    authorNode.className = 'item-author';
    authorNode.textContent = createdByName;

    const messageNode = document.createElement('p');
    messageNode.className = 'item-message';
    messageNode.textContent = messagePreview ? `Message: ${messagePreview}` : 'Message: aucun';

    const shortcutNode = document.createElement('p');
    shortcutNode.className = 'item-shortcut';
    shortcutNode.textContent = `Raccourci: ${params.shortcuts.length > 0 ? params.shortcuts.join(', ') : 'aucun'}`;

    const actionsNode = document.createElement('div');
    actionsNode.className = 'item-actions';

    const playButton = document.createElement('button');
    playButton.type = 'button';
    playButton.textContent = 'Jouer';
    playButton.addEventListener('click', () => {
      params.onTrigger();
    });

    const renameButton = document.createElement('button');
    renameButton.type = 'button';
    renameButton.className = 'ghost';
    renameButton.textContent = 'Éditer';
    renameButton.addEventListener('click', () => {
      params.onRename();
    });

    const bindButton = document.createElement('button');
    bindButton.type = 'button';
    bindButton.className = 'ghost';
    bindButton.textContent = 'Macro';
    bindButton.addEventListener('click', () => {
      params.onBind();
    });

    const deleteButton = document.createElement('button');
    deleteButton.type = 'button';
    deleteButton.className = 'danger';
    deleteButton.textContent = 'Suppr.';
    deleteButton.addEventListener('click', () => {
      params.onDelete();
    });

    actionsNode.appendChild(playButton);
    actionsNode.appendChild(renameButton);
    actionsNode.appendChild(bindButton);
    actionsNode.appendChild(deleteButton);

    card.appendChild(titleNode);
    card.appendChild(subNode);
    card.appendChild(authorNode);
    card.appendChild(messageNode);
    card.appendChild(shortcutNode);
    card.appendChild(actionsNode);

    // Preserve dataset availability for future incremental migrations/debug selectors.
    if (itemId) {
      card.dataset.itemId = itemId;
    }

    return card;
  }

  function buildPreviewMetaText(params: BuildPreviewMetaTextParams): string {
    return `${toMediaKindLabel(params.mediaKind)} | Message: ${params.hasMessage ? 'oui' : 'non'} | Raccourci: ${
      params.shortcuts.length > 0 ? params.shortcuts.join(', ') : 'aucun'
    }`;
  }

  return {
    createEmptyStateNode,
    createPreviewMediaNode,
    createItemCard,
    buildPreviewMetaText
  };
}
