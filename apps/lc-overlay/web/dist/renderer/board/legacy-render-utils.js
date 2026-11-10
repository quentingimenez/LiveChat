function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function toTrimmedString(value) {
    return `${value || ''}`.trim();
}
function toTextLabel(value, fallback) {
    const normalized = toTrimmedString(value);
    return normalized || fallback;
}
function toMediaKindLabel(value) {
    const normalized = toTrimmedString(value);
    return normalized || 'MEDIA';
}
export function createBoardLegacyRenderUtils() {
    function createEmptyStateNode(params) {
        const emptyNode = document.createElement('p');
        emptyNode.className = 'preview-empty';
        emptyNode.textContent = params.text;
        return emptyNode;
    }
    function createPreviewMediaNode(params) {
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
    function createItemCard(params) {
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
    function buildPreviewMetaText(params) {
        return `${toMediaKindLabel(params.mediaKind)} | Message: ${params.hasMessage ? 'oui' : 'non'} | Raccourci: ${params.shortcuts.length > 0 ? params.shortcuts.join(', ') : 'aucun'}`;
    }
    return {
        createEmptyStateNode,
        createPreviewMediaNode,
        createItemCard,
        buildPreviewMetaText
    };
}
