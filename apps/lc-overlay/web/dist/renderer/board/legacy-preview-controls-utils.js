export function createBoardLegacyPreviewControlsUtils() {
    function createPreviewMessageEditor(params) {
        const normalizedInitialMessage = `${params.initialMessage || ''}`.trim();
        const rootNode = document.createElement('div');
        rootNode.className = 'preview-message-editor';
        const messageLabelNode = document.createElement('label');
        messageLabelNode.className = 'preview-message-label';
        messageLabelNode.textContent = 'Message du mème';
        const inputNode = document.createElement('textarea');
        inputNode.className = 'preview-message-input';
        inputNode.rows = 2;
        inputNode.maxLength = 500;
        inputNode.placeholder = 'Message overlay (optionnel)';
        inputNode.spellcheck = true;
        inputNode.value = normalizedInitialMessage;
        inputNode.addEventListener('input', () => {
            params.onInput(inputNode);
        });
        inputNode.addEventListener('blur', () => {
            params.onBlur(inputNode);
        });
        const messageRowNode = document.createElement('div');
        messageRowNode.className = 'preview-message-row';
        messageRowNode.appendChild(inputNode);
        rootNode.appendChild(messageLabelNode);
        rootNode.appendChild(messageRowNode);
        return {
            rootNode,
            inputNode,
            initialMessage: normalizedInitialMessage
        };
    }
    function createActionButton(text, options) {
        const buttonNode = document.createElement('button');
        buttonNode.type = 'button';
        buttonNode.textContent = text;
        if (options.className) {
            buttonNode.className = options.className;
        }
        buttonNode.addEventListener('click', () => {
            options.onClick();
        });
        return buttonNode;
    }
    function createPreviewControls(params) {
        const controlsNode = document.createElement('div');
        controlsNode.className = 'preview-controls';
        controlsNode.appendChild(createActionButton('Jouer', {
            onClick: params.onPlay
        }));
        controlsNode.appendChild(createActionButton('Éditer', {
            className: 'ghost',
            onClick: params.onRename
        }));
        controlsNode.appendChild(createActionButton('Assigner un raccourci', {
            className: 'ghost',
            onClick: params.onBindShortcut
        }));
        controlsNode.appendChild(createActionButton('Supprimer raccourci', {
            className: 'ghost',
            onClick: params.onClearShortcut
        }));
        controlsNode.appendChild(createActionButton('Supprimer', {
            className: 'danger',
            onClick: params.onDelete
        }));
        return controlsNode;
    }
    return {
        createPreviewMessageEditor,
        createPreviewControls
    };
}
