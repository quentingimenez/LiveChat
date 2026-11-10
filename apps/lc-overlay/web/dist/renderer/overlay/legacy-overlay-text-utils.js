function toRecord(value) {
    return typeof value === 'object' && value !== null ? value : {};
}
function toTrimmedString(value) {
    return typeof value === 'string' ? value.trim() : '';
}
function resolveOverlayText(payload) {
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
export function createOverlayLegacyTextUtils() {
    function resolveAuthorInfo(payload) {
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
    function createAvatarNode(authorName, authorImage) {
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
    function createAuthorNode(authorName, authorImage) {
        const authorNode = document.createElement('div');
        authorNode.className = 'overlay-author';
        authorNode.appendChild(createAvatarNode(authorName, authorImage));
        const authorNameNode = document.createElement('div');
        authorNameNode.className = 'overlay-author-name';
        authorNameNode.textContent = authorName;
        authorNode.appendChild(authorNameNode);
        return authorNode;
    }
    function applyMediaHeaderAuthor(params) {
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
    function applyMediaFooterText(params) {
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
    function applyOverlayInfo(params) {
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
