(() => {
  const statusNode = document.getElementById('status');
  const countPillNode = document.getElementById('count-pill');
  const itemsListNode = document.getElementById('items-list');
  const previewStageNode = document.getElementById('preview-stage');
  const selectedTitleNode = document.getElementById('selected-title');
  const selectedMetaNode = document.getElementById('selected-meta');
  const searchForm = document.getElementById('search-form');
  const searchInput = document.getElementById('search-input');
  const stopPlaybackButton = document.getElementById('stop-playback-button');
  const refreshButton = document.getElementById('refresh-button');
  const addLinkButton = document.getElementById('add-link-button');
  const captureOverlay = document.getElementById('capture-overlay');
  const captureCurrentNode = document.getElementById('capture-current');
  const captureCancelButton = document.getElementById('capture-cancel');
  const captureSaveButton = document.getElementById('capture-save');
  const addOverlay = document.getElementById('add-overlay');
  const addForm = document.getElementById('add-form');
  const addUrlInput = document.getElementById('add-url-input');
  const addTitleInput = document.getElementById('add-title-input');
  const addMessageInput = document.getElementById('add-message-input');
  const addRefreshInput = document.getElementById('add-refresh-input');
  const addCancelButton = document.getElementById('add-cancel');
  const renameOverlay = document.getElementById('rename-overlay');
  const renameForm = document.getElementById('rename-form');
  const renameInput = document.getElementById('rename-input');
  const renameMessageInput = document.getElementById('rename-message-input');
  const renameCancelButton = document.getElementById('rename-cancel');
  const deleteOverlay = document.getElementById('delete-overlay');
  const deleteMessage = document.getElementById('delete-message');
  const deleteCancelButton = document.getElementById('delete-cancel');
  const deleteConfirmButton = document.getElementById('delete-confirm');

  const state = {
    config: null,
    bindings: {},
    items: [],
    total: 0,
    selectedId: null,
    selectedItem: null,
    captureItemId: null,
    capturePendingAccelerator: null,
    capturePendingDisplay: '',
    resolveAddDialog: null,
    resolveRenameDialog: null,
    resolveDeleteDialog: null,
    search: '',
    statusTimeoutId: null,
    searchDebounceTimeoutId: null,
    itemsLoadRequestId: 0,
    previewMediaKey: null,
    previewMessageAutosaveTimeoutId: null,
    previewMessageEditor: null,
  };
  const VOLUME_CURVE_GAMMA = 2.2;
  const INSTANT_SEARCH_DEBOUNCE_MS = 180;
  const PREVIEW_MESSAGE_AUTOSAVE_DEBOUNCE_MS = 700;

  const clearStatusTimer = () => {
    if (state.statusTimeoutId === null) {
      return;
    }

    clearTimeout(state.statusTimeoutId);
    state.statusTimeoutId = null;
  };

  const setStatus = (message, variant = 'success', options = {}) => {
    const autoDismissMs =
      typeof options?.autoDismissMs === 'number' && Number.isFinite(options.autoDismissMs)
        ? Math.max(0, Math.floor(options.autoDismissMs))
        : 5000;

    clearStatusTimer();

    // Force a hide/show cycle so a repeated toast still appears as a new one.
    statusNode.classList.add('hidden');
    statusNode.classList.remove('error', 'success');
    void statusNode.offsetWidth;

    statusNode.textContent = message;
    statusNode.classList.remove('hidden');
    statusNode.classList.add(variant);

    if (autoDismissMs > 0) {
      state.statusTimeoutId = setTimeout(() => {
        clearStatus();
      }, autoDismissMs);
    }
  };

  const clearStatus = () => {
    clearStatusTimer();
    statusNode.textContent = '';
    statusNode.classList.add('hidden');
    statusNode.classList.remove('error', 'success');
  };

  const clearSearchDebounce = () => {
    if (state.searchDebounceTimeoutId === null) {
      return;
    }

    clearTimeout(state.searchDebounceTimeoutId);
    state.searchDebounceTimeoutId = null;
  };

  const clearPreviewMessageAutosave = () => {
    if (state.previewMessageAutosaveTimeoutId === null) {
      return;
    }

    clearTimeout(state.previewMessageAutosaveTimeoutId);
    state.previewMessageAutosaveTimeoutId = null;
  };

  const resetPreviewMessageEditor = () => {
    clearPreviewMessageAutosave();
    state.previewMessageEditor = null;
  };

  const normalizeMediaKind = (value) => {
    const normalized = `${value || ''}`.trim().toLowerCase();

    if (normalized === 'image' || normalized === 'audio' || normalized === 'video') {
      return normalized;
    }

    return 'video';
  };

  const toSafeDateLabel = (value) => {
    const parsed = new Date(value);

    if (Number.isNaN(parsed.getTime())) {
      return 'Date inconnue';
    }

    return parsed.toLocaleString('fr-FR');
  };

  const toCardTitle = (item) => {
    const rawTitle = `${item?.title || ''}`.trim();

    if (rawTitle) {
      return rawTitle;
    }

    const source = `${item?.media?.sourceUrl || ''}`.trim();
    if (!source) {
      return `${item?.mediaAssetId || 'Media'}`;
    }

    try {
      const url = new URL(source);
      return `${url.hostname}${url.pathname}`;
    } catch {
      return source;
    }
  };

  const toMessagePreview = (value, maxLength = 120) => {
    const normalized = `${value || ''}`.trim().replace(/\s+/g, ' ');

    if (!normalized) {
      return '';
    }

    if (normalized.length <= maxLength) {
      return normalized;
    }

    return `${normalized.slice(0, Math.max(1, maxLength - 1))}…`;
  };

  const buildAuthedUrl = (pathname, options = {}) => {
    const baseUrl = `${state.config?.serverUrl || ''}`.trim();
    const token = `${state.config?.clientToken || ''}`.trim();

    const url = new URL(pathname, baseUrl);

    if (token) {
      url.searchParams.set('token', token);
    }

    for (const [key, value] of Object.entries(options)) {
      if (value === null || value === undefined || value === '') {
        continue;
      }

      url.searchParams.set(key, `${value}`);
    }

    return url;
  };

  const clamp01 = (value) => {
    if (typeof value !== 'number' || !Number.isFinite(value)) {
      return 1;
    }

    return Math.min(1, Math.max(0, value));
  };

  const toPerceptualGain = (linearVolume) => {
    const normalized = clamp01(linearVolume);
    return Math.pow(normalized, VOLUME_CURVE_GAMMA);
  };

  const applyPreviewVolume = (root = previewStageNode) => {
    if (!root) {
      return;
    }

    const gain = toPerceptualGain(state.config?.volume);
    const mediaElements = root.querySelectorAll('audio,video');
    mediaElements.forEach((element) => {
      element.volume = gain;
      element.muted = false;
    });
  };

  const findSelectedItem = () => {
    if (typeof state.selectedId === 'string' && state.selectedId.trim() !== '') {
      const fromList = state.items.find((item) => item.id === state.selectedId) || null;
      if (fromList) {
        state.selectedItem = fromList;
        return fromList;
      }
    }

    if (
      state.selectedItem &&
      typeof state.selectedItem.id === 'string' &&
      (!state.selectedId || state.selectedItem.id === state.selectedId)
    ) {
      return state.selectedItem;
    }

    return null;
  };

  const isHttpUrl = (value) => {
    const raw = `${value || ''}`.trim();

    if (!raw) {
      return false;
    }

    try {
      const parsed = new URL(raw);
      return parsed.protocol === 'https:' || parsed.protocol === 'http:';
    } catch {
      return false;
    }
  };

  const isAddDialogReady = () => {
    return (
      addLinkButton instanceof HTMLElement &&
      addOverlay instanceof HTMLElement &&
      addForm instanceof HTMLFormElement &&
      addUrlInput instanceof HTMLInputElement &&
      addTitleInput instanceof HTMLInputElement &&
      addMessageInput instanceof HTMLTextAreaElement &&
      addRefreshInput instanceof HTMLInputElement &&
      addCancelButton instanceof HTMLElement
    );
  };

  const hideAddDialog = () => {
    if (addOverlay instanceof HTMLElement) {
      addOverlay.classList.add('hidden');
    }
  };

  const closeAddDialog = (value) => {
    hideAddDialog();

    if (typeof state.resolveAddDialog !== 'function') {
      return;
    }

    const resolver = state.resolveAddDialog;
    state.resolveAddDialog = null;
    resolver(value);
  };

  const openAddDialog = () => {
    if (!isAddDialogReady()) {
      const fallbackUrl = window.prompt('Lien du mème :');

      if (fallbackUrl === null) {
        return Promise.resolve(null);
      }

      const normalizedFallbackUrl = fallbackUrl.trim();

      if (!isHttpUrl(normalizedFallbackUrl)) {
        setStatus('Lien invalide. Utilise une URL complète en http(s).', 'error');
        return Promise.resolve(null);
      }

      const fallbackTitle = window.prompt('Nom du mème (optionnel) :', '') || '';
      const fallbackMessage = window.prompt('Message overlay (optionnel):', '') || '';

      return Promise.resolve({
        url: normalizedFallbackUrl,
        title: fallbackTitle.trim(),
        message: fallbackMessage.trim(),
        forceRefresh: false,
      });
    }

    closeAddDialog(null);
    addUrlInput.value = '';
    addTitleInput.value = '';
    addMessageInput.value = '';
    addRefreshInput.checked = false;
    addOverlay.classList.remove('hidden');
    addUrlInput.focus();

    return new Promise((resolve) => {
      state.resolveAddDialog = resolve;
    });
  };

  const isDeleteDialogReady = () => {
    return (
      deleteOverlay instanceof HTMLElement &&
      deleteMessage instanceof HTMLElement &&
      deleteCancelButton instanceof HTMLElement &&
      deleteConfirmButton instanceof HTMLElement
    );
  };

  const hideDeleteDialog = () => {
    if (deleteOverlay instanceof HTMLElement) {
      deleteOverlay.classList.add('hidden');
    }
  };

  const closeDeleteDialog = (value) => {
    hideDeleteDialog();

    if (typeof state.resolveDeleteDialog !== 'function') {
      return;
    }

    const resolver = state.resolveDeleteDialog;
    state.resolveDeleteDialog = null;
    resolver(value);
  };

  const openDeleteDialog = (item) => {
    const title = toCardTitle(item);

    if (!isDeleteDialogReady()) {
      return Promise.resolve(window.confirm(`Supprimer "${title}" de la mème board ?`));
    }

    closeDeleteDialog(false);
    deleteMessage.textContent = `Tu vas supprimer "${title}" de la mème board. Cette action est irréversible.`;
    deleteOverlay.classList.remove('hidden');
    deleteConfirmButton.focus();

    return new Promise((resolve) => {
      state.resolveDeleteDialog = resolve;
    });
  };

  const isRenameDialogReady = () => {
    return (
      renameOverlay instanceof HTMLElement &&
      renameForm instanceof HTMLFormElement &&
      renameInput instanceof HTMLInputElement &&
      renameMessageInput instanceof HTMLTextAreaElement &&
      renameCancelButton instanceof HTMLElement
    );
  };

  const hideRenameDialog = () => {
    if (renameOverlay instanceof HTMLElement) {
      renameOverlay.classList.add('hidden');
    }
  };

  const closeRenameDialog = (value) => {
    hideRenameDialog();

    if (typeof state.resolveRenameDialog !== 'function') {
      return;
    }

    const resolver = state.resolveRenameDialog;
    state.resolveRenameDialog = null;
    resolver(value);
  };

  const openRenameDialog = (title, message) => {
    if (!isRenameDialogReady()) {
      const fallbackTitle = window.prompt('Nom du mème (laisser vide pour enlever le nom) :', title);

      if (fallbackTitle === null) {
        return Promise.resolve(null);
      }

      const fallbackMessage = window.prompt(
        'Message overlay (laisser vide pour enlever le message):',
        `${message || ''}`.trim(),
      );

      if (fallbackMessage === null) {
        return Promise.resolve(null);
      }

      return Promise.resolve({
        title: fallbackTitle,
        message: fallbackMessage,
      });
    }

    closeRenameDialog(null);

    renameInput.value = title;
    renameMessageInput.value = `${message || ''}`.trim();
    renameOverlay.classList.remove('hidden');
    renameInput.focus();
    renameInput.select();

    return new Promise((resolve) => {
      state.resolveRenameDialog = resolve;
    });
  };

  const bindBackdropClose = (overlayNode, closeDialog) => {
    if (!(overlayNode instanceof HTMLElement) || typeof closeDialog !== 'function') {
      return;
    }

    let pointerStartedOnBackdrop = false;

    overlayNode.addEventListener('mousedown', (event) => {
      pointerStartedOnBackdrop = event.target === overlayNode;
    });

    overlayNode.addEventListener('click', (event) => {
      const clickedBackdrop = event.target === overlayNode;

      if (clickedBackdrop && pointerStartedOnBackdrop) {
        closeDialog();
      }

      pointerStartedOnBackdrop = false;
    });
  };

  const getItemShortcuts = (itemId) => {
    const shortcuts = [];

    for (const [accelerator, mappedItemId] of Object.entries(state.bindings)) {
      if (mappedItemId === itemId) {
        shortcuts.push(accelerator);
      }
    }

    shortcuts.sort((left, right) => left.localeCompare(right, undefined, { sensitivity: 'base' }));

    return shortcuts;
  };

  const hasPendingPreviewMessageChanges = (editor = state.previewMessageEditor) => {
    if (!editor || !(editor.inputNode instanceof HTMLTextAreaElement)) {
      return false;
    }

    const draftValue = `${editor.inputNode.value || ''}`.trim();
    const savedValue = `${editor.lastSavedMessage || ''}`.trim();
    return draftValue !== savedValue;
  };

  const schedulePreviewMessageAutosave = (delayMs = PREVIEW_MESSAGE_AUTOSAVE_DEBOUNCE_MS) => {
    clearPreviewMessageAutosave();

    const editor = state.previewMessageEditor;
    if (!editor) {
      return;
    }

    state.previewMessageAutosaveTimeoutId = setTimeout(() => {
      state.previewMessageAutosaveTimeoutId = null;
      void savePreviewMessage();
    }, Math.max(0, Math.floor(delayMs)));
  };

  const savePreviewMessage = async (options = {}) => {
    const editor = options?.editor || state.previewMessageEditor;
    const waitForInFlight = options?.waitForInFlight !== false;
    const keepalive = options?.keepalive === true;
    const silent = options?.silent === true;

    if (!editor || !(editor.inputNode instanceof HTMLTextAreaElement)) {
      return false;
    }

    const itemId = `${editor.itemId || ''}`.trim();
    if (!itemId) {
      return false;
    }

    if (editor.saving) {
      if (waitForInFlight && editor.inFlightSavePromise) {
        try {
          await editor.inFlightSavePromise;
        } catch {
          // Ignore: the save call already reported the error.
        }
      } else {
        return !hasPendingPreviewMessageChanges(editor);
      }
    }

    const nextMessage = `${editor.inputNode.value || ''}`.trim();
    const previousSavedMessage = `${editor.lastSavedMessage || ''}`.trim();

    if (nextMessage === previousSavedMessage) {
      return true;
    }

    let nextTitle = '';
    const selected = findSelectedItem();
    if (selected && `${selected.id || ''}` === itemId) {
      nextTitle = `${selected?.title || ''}`.trim();
    } else {
      const inList = (Array.isArray(state.items) ? state.items : []).find((entry) => `${entry?.id || ''}` === itemId);
      nextTitle = `${inList?.title || ''}`.trim();
    }

    editor.saving = true;
    const savePromise = (async () => {
      try {
        await patchItemMetadata(
          itemId,
          {
            title: nextTitle,
            message: nextMessage,
          },
          { keepalive },
        );

        editor.lastSavedMessage = nextMessage;
        applyLocalItemMetadata(itemId, nextTitle, nextMessage);
        renderList();
        return true;
      } catch (error) {
        if (!silent) {
          setStatus(`Enregistrement du message impossible: ${error?.message || error}`, 'error');
        }
        return false;
      }
    })();

    editor.inFlightSavePromise = savePromise;

    try {
      return await savePromise;
    } finally {
      if (editor.inFlightSavePromise === savePromise) {
        editor.inFlightSavePromise = null;
      }
      editor.saving = false;

      if (state.previewMessageEditor === editor && hasPendingPreviewMessageChanges(editor)) {
        schedulePreviewMessageAutosave(280);
      }
    }
  };

  const flushPreviewMessageChanges = async (options = {}) => {
    clearPreviewMessageAutosave();

    const editor = options?.editor || state.previewMessageEditor;
    if (!editor || !(editor.inputNode instanceof HTMLTextAreaElement)) {
      return true;
    }

    // Handle race: save in progress + user typed again while request was pending.
    for (let attempt = 0; attempt < 2; attempt += 1) {
      const saved = await savePreviewMessage({
        editor,
        waitForInFlight: true,
        keepalive: options?.keepalive === true,
        silent: options?.silent === true,
      });

      if (!hasPendingPreviewMessageChanges(editor)) {
        return saved;
      }

      if (!saved) {
        return false;
      }
    }

    if (!options?.silent) {
      setStatus("Le message n'a pas pu être sauvegardé, action annulée.", 'error');
    }
    return false;
  };

  const renderPreview = () => {
    const selectedItem = findSelectedItem();

    if (!selectedItem) {
      if (hasPendingPreviewMessageChanges()) {
        const detachedEditor = state.previewMessageEditor;
        clearPreviewMessageAutosave();
        void savePreviewMessage({
          editor: detachedEditor,
          waitForInFlight: false,
          keepalive: true,
          silent: true,
        });
      }
      resetPreviewMessageEditor();
      state.previewMediaKey = null;
      previewStageNode.innerHTML = '';
      if (selectedTitleNode instanceof HTMLElement) {
        selectedTitleNode.textContent = '';
      }
      selectedMetaNode.textContent = 'Aucun élément sélectionné.';
      const emptyNode = document.createElement('p');
      emptyNode.className = 'preview-empty';
      emptyNode.textContent = 'Sélectionne un mème pour voir son média et gérer son raccourci.';
      previewStageNode.appendChild(emptyNode);
      return;
    }

    if (state.previewMessageEditor && `${state.previewMessageEditor.itemId || ''}` !== `${selectedItem.id || ''}`) {
      const detachedEditor = state.previewMessageEditor;
      if (hasPendingPreviewMessageChanges(detachedEditor)) {
        clearPreviewMessageAutosave();
        void savePreviewMessage({
          editor: detachedEditor,
          waitForInFlight: false,
          keepalive: true,
          silent: true,
        });
      }
      resetPreviewMessageEditor();
    }

    const mediaKind = normalizeMediaKind(selectedItem.media?.kind);
    const mediaUrl = buildAuthedUrl(`/overlay/meme-board/media/${selectedItem.mediaAssetId}`).toString();
    const mediaKey = `${selectedItem.id}|${selectedItem.mediaAssetId}|${mediaKind}|${mediaUrl}`;

    if (state.previewMediaKey !== mediaKey) {
      state.previewMediaKey = mediaKey;
      previewStageNode.innerHTML = '';

      let mediaNode;

      if (mediaKind === 'image') {
        mediaNode = document.createElement('img');
        mediaNode.src = mediaUrl;
        mediaNode.alt = toCardTitle(selectedItem);
        mediaNode.loading = 'lazy';
      } else if (mediaKind === 'audio') {
        mediaNode = document.createElement('audio');
        mediaNode.src = mediaUrl;
        mediaNode.controls = true;
        mediaNode.preload = 'metadata';
      } else {
        mediaNode = document.createElement('video');
        mediaNode.src = mediaUrl;
        mediaNode.controls = true;
        mediaNode.preload = 'metadata';
      }

      mediaNode.className = 'preview-media';

      const messageEditorNode = document.createElement('div');
      messageEditorNode.className = 'preview-message-editor';

      const messageLabelNode = document.createElement('label');
      messageLabelNode.className = 'preview-message-label';
      messageLabelNode.textContent = 'Message du mème';

      const messageInputNode = document.createElement('textarea');
      messageInputNode.className = 'preview-message-input';
      messageInputNode.rows = 2;
      messageInputNode.maxLength = 500;
      messageInputNode.placeholder = 'Message overlay (optionnel)';
      messageInputNode.spellcheck = true;
      messageInputNode.value = `${selectedItem?.message || ''}`.trim();

      const messageRowNode = document.createElement('div');
      messageRowNode.className = 'preview-message-row';

      messageRowNode.appendChild(messageInputNode);

      messageEditorNode.appendChild(messageLabelNode);
      messageEditorNode.appendChild(messageRowNode);

      state.previewMessageEditor = {
        itemId: `${selectedItem.id || ''}`.trim(),
        inputNode: messageInputNode,
        lastSavedMessage: `${messageInputNode.value || ''}`.trim(),
        saving: false,
        inFlightSavePromise: null,
      };

      messageInputNode.addEventListener('input', () => {
        const editor = state.previewMessageEditor;
        if (!editor || editor.inputNode !== messageInputNode) {
          return;
        }

        schedulePreviewMessageAutosave();
      });

      messageInputNode.addEventListener('blur', () => {
        schedulePreviewMessageAutosave(220);
      });

      const controlsNode = document.createElement('div');
      controlsNode.className = 'preview-controls';

      const triggerButton = document.createElement('button');
      triggerButton.type = 'button';
      triggerButton.textContent = 'Jouer';
      triggerButton.addEventListener('click', () => {
        void (async () => {
          const currentSelectedItem = findSelectedItem();
          if (!currentSelectedItem) {
            return;
          }
          await triggerItem(currentSelectedItem.id);
        })();
      });

      const renameButton = document.createElement('button');
      renameButton.type = 'button';
      renameButton.className = 'ghost';
      renameButton.textContent = 'Éditer';
      renameButton.addEventListener('click', () => {
        void (async () => {
          const currentSelectedItem = findSelectedItem();
          if (!currentSelectedItem) {
            return;
          }
          await renameItem(currentSelectedItem);
        })();
      });

      const bindButton = document.createElement('button');
      bindButton.type = 'button';
      bindButton.className = 'ghost';
      bindButton.textContent = 'Assigner un raccourci';
      bindButton.addEventListener('click', () => {
        void (async () => {
          const currentSelectedItem = findSelectedItem();
          if (!currentSelectedItem) {
            return;
          }
          const flushed = await flushPreviewMessageChanges();
          if (!flushed) {
            return;
          }
          beginCaptureForItem(currentSelectedItem.id);
        })();
      });

      const clearButton = document.createElement('button');
      clearButton.type = 'button';
      clearButton.className = 'ghost';
      clearButton.textContent = 'Supprimer raccourci';
      clearButton.addEventListener('click', () => {
        void (async () => {
          const currentSelectedItem = findSelectedItem();
          if (!currentSelectedItem) {
            return;
          }
          await clearShortcutForItem(currentSelectedItem.id);
        })();
      });

      const deleteButton = document.createElement('button');
      deleteButton.type = 'button';
      deleteButton.className = 'danger';
      deleteButton.textContent = 'Supprimer';
      deleteButton.addEventListener('click', () => {
        void (async () => {
          const currentSelectedItem = findSelectedItem();
          if (!currentSelectedItem) {
            return;
          }
          await deleteItem(currentSelectedItem);
        })();
      });

      controlsNode.appendChild(triggerButton);
      controlsNode.appendChild(renameButton);
      controlsNode.appendChild(bindButton);
      controlsNode.appendChild(clearButton);
      controlsNode.appendChild(deleteButton);

      previewStageNode.appendChild(mediaNode);
      previewStageNode.appendChild(messageEditorNode);
      previewStageNode.appendChild(controlsNode);
    }
    applyPreviewVolume();

    const shortcuts = getItemShortcuts(selectedItem.id);
    const hasMessage = toMessagePreview(selectedItem?.message).length > 0;
    if (selectedTitleNode instanceof HTMLElement) {
      selectedTitleNode.textContent = `${toCardTitle(selectedItem)}`;
    }
    selectedMetaNode.textContent = `${selectedItem.media?.kind || 'MEDIA'} | Message: ${
      hasMessage ? 'oui' : 'non'
    } | Raccourci: ${shortcuts.length > 0 ? shortcuts.join(', ') : 'aucun'}`;
  };

  const renderList = () => {
    itemsListNode.innerHTML = '';
    countPillNode.textContent = `${state.total}`;

    if (!Array.isArray(state.items) || state.items.length === 0) {
      const emptyNode = document.createElement('p');
      emptyNode.className = 'preview-empty';
      emptyNode.textContent = 'Aucun mème dans la board pour cette recherche.';
      itemsListNode.appendChild(emptyNode);
      renderPreview();
      return;
    }

    for (const item of state.items) {
      const card = document.createElement('article');
      card.className = `item-card${item.id === state.selectedId ? ' selected' : ''}`;

      card.addEventListener('click', (event) => {
        const target = event.target;
        if (target instanceof Element && target.closest('button')) {
          return;
        }

        void (async () => {
          if (`${state.selectedId || ''}` !== `${item.id}`) {
            const flushed = await flushPreviewMessageChanges();
            if (!flushed) {
              return;
            }
            clearStatus();
          }

          state.selectedId = item.id;
          state.selectedItem = item;
          renderList();
        })();
      });

      const titleNode = document.createElement('h3');
      titleNode.className = 'item-title';
      titleNode.textContent = toCardTitle(item);

      const subNode = document.createElement('p');
      subNode.className = 'item-subline';
      subNode.textContent = `${item.media?.kind || 'MEDIA'} | ${toSafeDateLabel(item.createdAt)}`;

      const authorNode = document.createElement('p');
      authorNode.className = 'item-author';
      authorNode.textContent = `${item.createdByName || 'Auteur inconnu'}`;

      const messageNode = document.createElement('p');
      messageNode.className = 'item-message';
      const messagePreview = toMessagePreview(item?.message);
      messageNode.textContent = messagePreview ? `Message: ${messagePreview}` : 'Message: aucun';

      const shortcutNode = document.createElement('p');
      shortcutNode.className = 'item-shortcut';
      const shortcuts = getItemShortcuts(item.id);
      shortcutNode.textContent = `Raccourci: ${shortcuts.length > 0 ? shortcuts.join(', ') : 'aucun'}`;

      const actionsNode = document.createElement('div');
      actionsNode.className = 'item-actions';

      const playButton = document.createElement('button');
      playButton.type = 'button';
      playButton.textContent = 'Jouer';
      playButton.addEventListener('click', () => {
        void triggerItem(item.id);
      });

      const renameButton = document.createElement('button');
      renameButton.type = 'button';
      renameButton.className = 'ghost';
      renameButton.textContent = 'Éditer';
      renameButton.addEventListener('click', () => {
        void renameItem(item);
      });

      const bindButton = document.createElement('button');
      bindButton.type = 'button';
      bindButton.className = 'ghost';
      bindButton.textContent = 'Macro';
      bindButton.addEventListener('click', () => {
        void (async () => {
          const flushed = await flushPreviewMessageChanges();
          if (!flushed) {
            return;
          }
          beginCaptureForItem(item.id);
        })();
      });

      const deleteButton = document.createElement('button');
      deleteButton.type = 'button';
      deleteButton.className = 'danger';
      deleteButton.textContent = 'Suppr.';
      deleteButton.addEventListener('click', () => {
        void deleteItem(item);
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

      itemsListNode.appendChild(card);
    }

    renderPreview();
  };

  const fetchItems = async (searchQuery) => {
    const endpoint = buildAuthedUrl('/overlay/meme-board/items', {
      limit: 150,
      offset: 0,
      q: searchQuery,
    });

    const response = await fetch(endpoint.toString(), {
      method: 'GET',
      cache: 'no-store',
    });

    const payload = await response.json().catch(() => null);

    if (!response.ok) {
      const message = payload?.error ? `Chargement board impossible: ${payload.error}` : 'Chargement board impossible.';
      throw new Error(message);
    }

    const items = Array.isArray(payload?.items) ? payload.items : [];
    const total = typeof payload?.total === 'number' ? payload.total : items.length;

    return {
      items,
      total,
    };
  };

  const patchItemMetadata = async (itemId, payload, options = {}) => {
    const endpoint = buildAuthedUrl(`/overlay/meme-board/items/${itemId}`);
    const response = await fetch(endpoint.toString(), {
      method: 'PATCH',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
      keepalive: options?.keepalive === true,
    });

    if (!response.ok) {
      const body = await response.json().catch(() => null);
      throw new Error(body?.error || `HTTP_${response.status}`);
    }
  };

  const applyLocalItemMetadata = (itemId, nextTitle, nextMessage) => {
    let updated = false;

    state.items = (Array.isArray(state.items) ? state.items : []).map((entry) => {
      if (`${entry?.id || ''}` !== itemId) {
        return entry;
      }

      updated = true;
      return {
        ...entry,
        title: nextTitle,
        message: nextMessage,
      };
    });

    if (state.selectedItem && `${state.selectedItem.id || ''}` === itemId) {
      state.selectedItem = {
        ...state.selectedItem,
        title: nextTitle,
        message: nextMessage,
      };
      updated = true;
    }

    return updated;
  };

  const loadItemsAndRender = async () => {
    const requestId = state.itemsLoadRequestId + 1;
    state.itemsLoadRequestId = requestId;

    try {
      const fetched = await fetchItems(state.search);
      if (requestId !== state.itemsLoadRequestId) {
        return;
      }

      state.items = fetched.items;
      state.total = fetched.total;

      const selectedInList =
        typeof state.selectedId === 'string' && state.selectedId.trim() !== ''
          ? state.items.find((item) => item.id === state.selectedId) || null
          : null;

      if (selectedInList) {
        state.selectedItem = selectedInList;
      } else if (!state.selectedId && !state.selectedItem && state.items.length > 0) {
        state.selectedId = state.items[0].id;
        state.selectedItem = state.items[0];
      }

      await syncBindingsWithBoardItems();
      if (requestId !== state.itemsLoadRequestId) {
        return;
      }

      renderList();
      clearStatus();
    } catch (error) {
      setStatus(error?.message || 'Erreur de chargement de la mème board.', 'error');
    }
  };

  const runSearchNow = async () => {
    clearSearchDebounce();
    const flushed = await flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }
    state.search = `${searchInput.value || ''}`.trim();
    await loadItemsAndRender();
  };

  const scheduleInstantSearch = () => {
    clearSearchDebounce();

    state.searchDebounceTimeoutId = setTimeout(() => {
      state.searchDebounceTimeoutId = null;
      void (async () => {
        const flushed = await flushPreviewMessageChanges();
        if (!flushed) {
          return;
        }

        state.search = `${searchInput.value || ''}`.trim();
        await loadItemsAndRender();
      })();
    }, INSTANT_SEARCH_DEBOUNCE_MS);
  };

  const addItemFromLink = async (payload) => {
    try {
      const flushed = await flushPreviewMessageChanges();
      if (!flushed) {
        return;
      }

      setStatus('Ajout du mème en cours...', 'success');

      const endpoint = buildAuthedUrl('/overlay/meme-board/items');
      const response = await fetch(endpoint.toString(), {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          url: payload.url,
          title: payload.title,
          message: payload.message,
          forceRefresh: payload.forceRefresh,
        }),
      });

      const body = await response.json().catch(() => null);

      if (!response.ok) {
        const reason = body?.message || body?.error || `HTTP_${response.status}`;
        throw new Error(reason);
      }

      if (body?.item?.id) {
        state.selectedId = body.item.id;
        state.selectedItem = body.item;
      }

      await loadItemsAndRender();
      setStatus(body?.created === false ? 'Mème déjà présent dans la board.' : 'Mème ajouté dans la board.', 'success');
    } catch (error) {
      setStatus(`Ajout impossible: ${error?.message || error}`, 'error');
    }
  };

  const persistBindings = async (nextBindings) => {
    const result = await window.livechatOverlay.setMemeBindings({
      bindings: nextBindings,
    });

    if (!result || result.ok !== true) {
      const failed = Array.isArray(result?.failedAccelerators) && result.failedAccelerators.length > 0
        ? ` (${result.failedAccelerators.join(', ')})`
        : '';

      throw new Error(`Impossible d'appliquer les raccourcis${failed}`);
    }

    state.bindings = result.bindings || {};
  };

  const triggerItem = async (itemId) => {
    try {
      const flushed = await flushPreviewMessageChanges();
      if (!flushed) {
        return;
      }

      const result = await window.livechatOverlay.triggerMeme({
        itemId,
        trigger: 'ui',
      });

      if (!result?.ok) {
        throw new Error(result?.reason || 'socket_offline');
      }

      setStatus('Mème envoyé dans la file.', 'success');
    } catch (error) {
      setStatus(`Échec trigger : ${error?.message || error}`, 'error');
    }
  };

  const stopCurrentPlayback = async () => {
    try {
      const result = await window.livechatOverlay.stopMemePlayback();

      if (!result?.ok) {
        throw new Error(result?.reason || 'socket_offline');
      }

      setStatus('Lecture en cours stoppée.', 'success');
    } catch (error) {
      setStatus(`Stop impossible: ${error?.message || error}`, 'error');
    }
  };

  const removeBindingsForItem = (itemId) => {
    const nextBindings = { ...state.bindings };
    let removedCount = 0;

    for (const [accelerator, mappedItemId] of Object.entries(nextBindings)) {
      if (mappedItemId === itemId) {
        delete nextBindings[accelerator];
        removedCount += 1;
      }
    }

    return {
      nextBindings,
      removedCount,
    };
  };

  const syncBindingsWithBoardItems = async () => {
    const validItemIds = new Set(
      (Array.isArray(state.items) ? state.items : [])
        .map((item) => `${item?.id || ''}`.trim())
        .filter(Boolean),
    );
    const nextBindings = {};
    let removedCount = 0;

    for (const [accelerator, mappedItemId] of Object.entries(state.bindings || {})) {
      if (validItemIds.has(mappedItemId)) {
        nextBindings[accelerator] = mappedItemId;
        continue;
      }

      removedCount += 1;
    }

    if (removedCount === 0) {
      return;
    }

    await persistBindings(nextBindings);
  };

  const clearShortcutForItem = async (itemId) => {
    const flushed = await flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    const { nextBindings, removedCount } = removeBindingsForItem(itemId);

    if (removedCount === 0) {
      setStatus('Aucun raccourci à retirer pour ce mème.', 'success');
      return;
    }

    try {
      await persistBindings(nextBindings);
      renderList();
      setStatus('Raccourci retiré.', 'success');
    } catch (error) {
      setStatus(error?.message || 'Impossible de retirer le raccourci.', 'error');
    }
  };

  const renameItem = async (item) => {
    const flushed = await flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    const currentTitle = `${item?.title || ''}`.trim();
    const currentMessage = `${item?.message || ''}`.trim();
    const nextMetadata = await openRenameDialog(currentTitle, currentMessage);

    if (nextMetadata === null) {
      return;
    }

    const nextTitle = `${nextMetadata?.title || ''}`.trim();
    const nextMessage = `${nextMetadata?.message || ''}`.trim();

    try {
      const endpoint = buildAuthedUrl(`/overlay/meme-board/items/${item.id}`);
      const response = await fetch(endpoint.toString(), {
        method: 'PATCH',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({
          title: nextTitle,
          message: nextMessage,
        }),
      });

      if (!response.ok) {
        const payload = await response.json().catch(() => null);
        throw new Error(payload?.error || `HTTP_${response.status}`);
      }

      await loadItemsAndRender();
      const hasTitle = nextTitle.length > 0;
      const hasMessage = nextMessage.length > 0;
      setStatus(
        hasTitle || hasMessage
          ? `Mème mis à jour (${hasTitle ? 'nom' : 'sans nom'} / ${hasMessage ? 'message' : 'sans message'}).`
          : 'Nom et message supprimés.',
        'success',
      );
    } catch (error) {
      setStatus(`Renommage impossible: ${error?.message || error}`, 'error');
    }
  };

  const deleteItem = async (item) => {
    const flushed = await flushPreviewMessageChanges();
    if (!flushed) {
      return;
    }

    const confirmed = await openDeleteDialog(item);

    if (!confirmed) {
      return;
    }

    try {
      const endpoint = buildAuthedUrl(`/overlay/meme-board/items/${item.id}`);
      const response = await fetch(endpoint.toString(), {
        method: 'DELETE',
      });

      if (!response.ok) {
        const payload = await response.json().catch(() => null);
        throw new Error(payload?.error || `HTTP_${response.status}`);
      }

      if (state.selectedId === item.id) {
        state.selectedId = null;
        state.selectedItem = null;
        state.previewMediaKey = null;
      }

      const { nextBindings, removedCount } = removeBindingsForItem(item.id);
      let shortcutCleanupError = null;

      if (removedCount > 0) {
        try {
          await persistBindings(nextBindings);
        } catch (error) {
          shortcutCleanupError = error;
        }
      }

      await loadItemsAndRender();

      if (shortcutCleanupError) {
        setStatus(
          `Mème supprimé, mais libération du raccourci impossible : ${
            shortcutCleanupError?.message || shortcutCleanupError
          }`,
          'error',
        );
        return;
      }

      setStatus(removedCount > 0 ? 'Mème supprimé. Raccourci libéré.' : 'Mème supprimé.', 'success');
    } catch (error) {
      setStatus(`Suppression impossible: ${error?.message || error}`, 'error');
    }
  };

  const isCaptureUiReady = () => {
    return (
      captureOverlay instanceof HTMLElement &&
      captureCurrentNode instanceof HTMLElement &&
      captureCancelButton instanceof HTMLElement &&
      captureSaveButton instanceof HTMLButtonElement
    );
  };

  const acceleratorTokenToLabel = (value) => {
    const token = `${value || ''}`.trim();
    const lowered = token.toLowerCase();

    if (lowered === 'commandorcontrol') {
      return 'Ctrl/Cmd';
    }

    if (lowered === 'super') {
      return 'Super';
    }

    if (lowered === 'alt') {
      return 'Alt';
    }

    if (lowered === 'shift') {
      return 'Shift';
    }

    if (token.startsWith('num')) {
      return `Num${token.slice(3)}`;
    }

    return token;
  };

  const formatAcceleratorDisplay = (accelerator) => {
    const raw = `${accelerator || ''}`.trim();

    if (!raw) {
      return '';
    }

    return raw
      .split('+')
      .map((token) => acceleratorTokenToLabel(token))
      .filter(Boolean)
      .join(' + ');
  };

  const keyEventToDisplay = (event) => {
    const parts = [];

    if (event.ctrlKey) {
      parts.push('Ctrl/Cmd');
    }

    if (event.altKey) {
      parts.push('Alt');
    }

    if (event.shiftKey) {
      parts.push('Shift');
    }

    if (event.metaKey) {
      parts.push('Super');
    }

    const key = normalizeKey(event.key, event.code);

    if (key) {
      parts.push(acceleratorTokenToLabel(key));
    }

    return parts.join(' + ');
  };

  const refreshCaptureUi = () => {
    if (!isCaptureUiReady()) {
      return;
    }

    captureCurrentNode.textContent = state.capturePendingDisplay || 'Aucune combinaison détectée.';
    captureSaveButton.disabled = !state.capturePendingAccelerator;
  };

  const commitCapturedShortcut = async () => {
    const itemId = state.captureItemId;
    const accelerator = `${state.capturePendingAccelerator || ''}`.trim();

    if (!itemId) {
      setStatus('Aucun mème cible pour ce raccourci.', 'error');
      return;
    }

    if (!accelerator) {
      setStatus('Aucune combinaison valide détectée.', 'error');
      return;
    }

    const nextBindings = { ...state.bindings };

    for (const [existingAccelerator, mappedItemId] of Object.entries(nextBindings)) {
      if (mappedItemId === itemId || existingAccelerator === accelerator) {
        delete nextBindings[existingAccelerator];
      }
    }

    nextBindings[accelerator] = itemId;

    try {
      await persistBindings(nextBindings);
      endCapture();
      renderList();
      setStatus(`Raccourci assigné : ${formatAcceleratorDisplay(accelerator)}`, 'success');
    } catch (error) {
      setStatus(error?.message || "Impossible d'assigner ce raccourci.", 'error');
    }
  };

  const beginCaptureForItem = (itemId) => {
    state.captureItemId = itemId;
    state.capturePendingAccelerator = null;
    state.capturePendingDisplay = '';
    refreshCaptureUi();
    captureOverlay.classList.remove('hidden');
    setStatus(
      'Capture clavier active : fais la combinaison puis clique "Arrêter et enregistrer" (Esc pour annuler).',
      'success',
    );
  };

  const endCapture = () => {
    state.captureItemId = null;
    state.capturePendingAccelerator = null;
    state.capturePendingDisplay = '';
    refreshCaptureUi();
    captureOverlay.classList.add('hidden');
  };

  const normalizeKeyFromCode = (eventCode) => {
    const code = `${eventCode || ''}`;

    if (!code) {
      return null;
    }

    if (/^Key[A-Z]$/.test(code)) {
      return code.slice(3);
    }

    if (/^Digit[0-9]$/.test(code)) {
      return code.slice(5);
    }

    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) {
      return code;
    }

    if (/^Numpad[0-9]$/.test(code)) {
      return `num${code.slice(6)}`;
    }

    const map = {
      Space: 'Space',
      Enter: 'Enter',
      Tab: 'Tab',
      Escape: 'Escape',
      Backspace: 'Backspace',
      Delete: 'Delete',
      Insert: 'Insert',
      Home: 'Home',
      End: 'End',
      PageUp: 'PageUp',
      PageDown: 'PageDown',
      ArrowUp: 'Up',
      ArrowDown: 'Down',
      ArrowLeft: 'Left',
      ArrowRight: 'Right',
      Minus: '-',
      Equal: '=',
      Comma: ',',
      Period: '.',
      Slash: '/',
      Semicolon: ';',
      Quote: "'",
      BracketLeft: '[',
      BracketRight: ']',
      Backslash: '\\',
      Backquote: '`',
    };

    return map[code] || null;
  };

  const normalizeKey = (eventKey, eventCode) => {
    const codeCandidate = normalizeKeyFromCode(eventCode);

    if (codeCandidate) {
      return codeCandidate;
    }

    const key = `${eventKey || ''}`;

    if (!key) {
      return null;
    }

    const lower = key.toLowerCase();

    if (lower === 'control' || lower === 'shift' || lower === 'alt' || lower === 'meta') {
      return null;
    }

    if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(key)) {
      return key.toUpperCase();
    }

    if (/^[a-z]$/i.test(key)) {
      return key.toUpperCase();
    }

    if (/^[0-9]$/.test(key)) {
      return key;
    }

    const map = {
      ' ': 'Space',
      spacebar: 'Space',
      enter: 'Enter',
      return: 'Enter',
      tab: 'Tab',
      escape: 'Escape',
      esc: 'Escape',
      backspace: 'Backspace',
      delete: 'Delete',
      insert: 'Insert',
      home: 'Home',
      end: 'End',
      pageup: 'PageUp',
      pagedown: 'PageDown',
      arrowup: 'Up',
      arrowdown: 'Down',
      arrowleft: 'Left',
      arrowright: 'Right',
      plus: '+',
      minus: '-',
    };

    return map[lower] || null;
  };

  const keyEventToAccelerator = (event) => {
    const key = normalizeKey(event.key, event.code);

    if (!key) {
      return null;
    }

    const modifiers = [];

    if (event.ctrlKey) {
      modifiers.push('CommandOrControl');
    }

    if (event.altKey) {
      modifiers.push('Alt');
    }

    if (event.shiftKey) {
      modifiers.push('Shift');
    }

    if (event.metaKey) {
      modifiers.push('Super');
    }

    if (modifiers.length === 0) {
      return null;
    }

    return [...modifiers, key].join('+');
  };

  const flushPreviewMessageOnLifecycleExit = () => {
    const editor = state.previewMessageEditor;
    if (!hasPendingPreviewMessageChanges(editor)) {
      return;
    }

    void flushPreviewMessageChanges({
      editor,
      keepalive: true,
      silent: true,
    });
  };

  window.addEventListener('keydown', (event) => {
    if (state.resolveAddDialog) {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeAddDialog(null);
      }

      return;
    }

    if (state.resolveRenameDialog) {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeRenameDialog(null);
      }

      return;
    }

    if (state.resolveDeleteDialog) {
      if (event.key === 'Escape') {
        event.preventDefault();
        closeDeleteDialog(false);
      } else if (event.key === 'Enter') {
        event.preventDefault();
        closeDeleteDialog(true);
      }

      return;
    }

    if (!state.captureItemId) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    if (event.key === 'Escape') {
      endCapture();
      clearStatus();
      return;
    }

    const accelerator = keyEventToAccelerator(event);
    const display = keyEventToDisplay(event);

    if (!accelerator) {
      if (!state.capturePendingAccelerator) {
        state.capturePendingDisplay = display || state.capturePendingDisplay;
      }

      refreshCaptureUi();
      setStatus('Combinaison en cours... ajoute une touche non-modificatrice pour valider.', 'success');
      return;
    }

    state.capturePendingAccelerator = accelerator;
    state.capturePendingDisplay = formatAcceleratorDisplay(accelerator);
    refreshCaptureUi();
    setStatus(`Combinaison détectée : ${formatAcceleratorDisplay(accelerator)}. Clique "Arrêter et enregistrer".`, 'success');
  });

  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') {
      flushPreviewMessageOnLifecycleExit();
    }
  });

  window.addEventListener('pagehide', () => {
    flushPreviewMessageOnLifecycleExit();
  });

  window.addEventListener('beforeunload', () => {
    flushPreviewMessageOnLifecycleExit();
  });

  if (isCaptureUiReady()) {
    captureCancelButton.addEventListener('click', () => {
      endCapture();
      clearStatus();
    });

    captureSaveButton.addEventListener('click', () => {
      void commitCapturedShortcut();
    });

    refreshCaptureUi();
  }

  if (isAddDialogReady()) {
    addForm.addEventListener('submit', (event) => {
      event.preventDefault();

      const url = `${addUrlInput.value || ''}`.trim();
      const title = `${addTitleInput.value || ''}`.trim();
      const message = `${addMessageInput.value || ''}`.trim();
      const forceRefresh = !!addRefreshInput.checked;

      if (!isHttpUrl(url)) {
        setStatus('Lien invalide. Utilise une URL complète en http(s).', 'error');
        addUrlInput.focus();
        return;
      }

      closeAddDialog({
        url,
        title,
        message,
        forceRefresh,
      });
    });

    addCancelButton.addEventListener('click', () => {
      closeAddDialog(null);
    });

    bindBackdropClose(addOverlay, () => {
      closeAddDialog(null);
    });
  }

  if (addLinkButton instanceof HTMLElement) {
    addLinkButton.addEventListener('click', () => {
      void (async () => {
        const payload = await openAddDialog();

        if (!payload) {
          return;
        }

        await addItemFromLink(payload);
      })();
    });
  }

  if (isRenameDialogReady()) {
    renameForm.addEventListener('submit', (event) => {
      event.preventDefault();
      closeRenameDialog({
        title: renameInput.value,
        message: renameMessageInput.value,
      });
    });

    renameCancelButton.addEventListener('click', () => {
      closeRenameDialog(null);
    });

    bindBackdropClose(renameOverlay, () => {
      closeRenameDialog(null);
    });
  }

  if (isDeleteDialogReady()) {
    deleteCancelButton.addEventListener('click', () => {
      closeDeleteDialog(false);
    });

    deleteConfirmButton.addEventListener('click', () => {
      closeDeleteDialog(true);
    });

    bindBackdropClose(deleteOverlay, () => {
      closeDeleteDialog(false);
    });
  }

  searchForm.addEventListener('submit', (event) => {
    event.preventDefault();
    void runSearchNow();
  });

  searchInput.addEventListener('input', () => {
    scheduleInstantSearch();
  });

  refreshButton.addEventListener('click', () => {
    void (async () => {
      const flushed = await flushPreviewMessageChanges();
      if (!flushed) {
        return;
      }
      await loadItemsAndRender();
    })();
  });

  if (stopPlaybackButton instanceof HTMLElement) {
    stopPlaybackButton.addEventListener('click', () => {
      void stopCurrentPlayback();
    });
  }

  const bootstrap = async () => {
    state.config = await window.livechatOverlay.getConfig();

    if (!state.config?.serverUrl || !state.config?.clientToken || !state.config?.guildId) {
      setStatus("Overlay non appairé. Ouvre la fenêtre d'appairage puis reconnecte.", 'error');
      refreshButton.disabled = true;
      searchInput.disabled = true;
      if (stopPlaybackButton instanceof HTMLButtonElement) {
        stopPlaybackButton.disabled = true;
      }
      if (addLinkButton instanceof HTMLButtonElement) {
        addLinkButton.disabled = true;
      }
      return;
    }

    const bindingsResult = await window.livechatOverlay.getMemeBindings();
    state.bindings = bindingsResult?.bindings || {};

    window.livechatOverlay.onSettings((nextSettings) => {
      state.config = {
        ...(state.config || {}),
        ...(nextSettings || {}),
      };
      applyPreviewVolume();
    });

    await loadItemsAndRender();
  };

  bootstrap().catch((error) => {
    setStatus(error?.message || 'Initialisation mème board impossible.', 'error');
  });
})();
