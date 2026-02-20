(() => {
  const statusNode = document.getElementById('status');
  const countPillNode = document.getElementById('count-pill');
  const itemsListNode = document.getElementById('items-list');
  const previewStageNode = document.getElementById('preview-stage');
  const selectedMetaNode = document.getElementById('selected-meta');
  const searchForm = document.getElementById('search-form');
  const searchInput = document.getElementById('search-input');
  const refreshButton = document.getElementById('refresh-button');
  const captureOverlay = document.getElementById('capture-overlay');

  const state = {
    config: null,
    bindings: {},
    items: [],
    total: 0,
    selectedId: null,
    captureItemId: null,
    search: '',
  };

  const setStatus = (message, variant = 'success') => {
    statusNode.textContent = message;
    statusNode.classList.remove('hidden', 'error', 'success');
    statusNode.classList.add(variant);
  };

  const clearStatus = () => {
    statusNode.textContent = '';
    statusNode.classList.add('hidden');
    statusNode.classList.remove('error', 'success');
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

  const findSelectedItem = () => {
    return state.items.find((item) => item.id === state.selectedId) || null;
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

  const renderPreview = () => {
    previewStageNode.innerHTML = '';

    const selectedItem = findSelectedItem();

    if (!selectedItem) {
      selectedMetaNode.textContent = 'Aucun element selectionne.';
      const emptyNode = document.createElement('p');
      emptyNode.className = 'preview-empty';
      emptyNode.textContent = 'Selectionne un meme pour voir son media et gerer son raccourci.';
      previewStageNode.appendChild(emptyNode);
      return;
    }

    const mediaKind = normalizeMediaKind(selectedItem.media?.kind);
    const mediaUrl = buildAuthedUrl(`/overlay/meme-board/media/${selectedItem.mediaAssetId}`).toString();

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

    const controlsNode = document.createElement('div');
    controlsNode.className = 'preview-controls';

    const triggerButton = document.createElement('button');
    triggerButton.type = 'button';
    triggerButton.textContent = 'Jouer';
    triggerButton.addEventListener('click', () => {
      void triggerItem(selectedItem.id);
    });

    const bindButton = document.createElement('button');
    bindButton.type = 'button';
    bindButton.className = 'ghost';
    bindButton.textContent = 'Assigner un raccourci';
    bindButton.addEventListener('click', () => {
      beginCaptureForItem(selectedItem.id);
    });

    const clearButton = document.createElement('button');
    clearButton.type = 'button';
    clearButton.className = 'ghost';
    clearButton.textContent = 'Supprimer raccourci';
    clearButton.addEventListener('click', () => {
      void clearShortcutForItem(selectedItem.id);
    });

    const deleteButton = document.createElement('button');
    deleteButton.type = 'button';
    deleteButton.className = 'danger';
    deleteButton.textContent = 'Supprimer ce meme';
    deleteButton.addEventListener('click', () => {
      void deleteItem(selectedItem);
    });

    controlsNode.appendChild(triggerButton);
    controlsNode.appendChild(bindButton);
    controlsNode.appendChild(clearButton);
    controlsNode.appendChild(deleteButton);

    previewStageNode.appendChild(mediaNode);
    previewStageNode.appendChild(controlsNode);

    const shortcuts = getItemShortcuts(selectedItem.id);
    selectedMetaNode.textContent = `${toCardTitle(selectedItem)} | ${selectedItem.media?.kind || 'MEDIA'} | Raccourci: ${
      shortcuts.length > 0 ? shortcuts.join(', ') : 'aucun'
    }`;
  };

  const renderList = () => {
    itemsListNode.innerHTML = '';
    countPillNode.textContent = `${state.total}`;

    if (!Array.isArray(state.items) || state.items.length === 0) {
      const emptyNode = document.createElement('p');
      emptyNode.className = 'preview-empty';
      emptyNode.textContent = 'Aucun meme dans la board pour cette recherche.';
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

        state.selectedId = item.id;
        renderList();
      });

      const titleNode = document.createElement('h3');
      titleNode.className = 'item-title';
      titleNode.textContent = toCardTitle(item);

      const subNode = document.createElement('p');
      subNode.className = 'item-subline';
      subNode.textContent = `${item.media?.kind || 'MEDIA'} | ${item.createdByName || 'Auteur inconnu'} | ${toSafeDateLabel(item.createdAt)}`;

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

      const bindButton = document.createElement('button');
      bindButton.type = 'button';
      bindButton.className = 'ghost';
      bindButton.textContent = 'Assigner';
      bindButton.addEventListener('click', () => {
        beginCaptureForItem(item.id);
      });

      const clearButton = document.createElement('button');
      clearButton.type = 'button';
      clearButton.className = 'ghost';
      clearButton.textContent = 'Retirer raccourci';
      clearButton.addEventListener('click', () => {
        void clearShortcutForItem(item.id);
      });

      const deleteButton = document.createElement('button');
      deleteButton.type = 'button';
      deleteButton.className = 'danger';
      deleteButton.textContent = 'Supprimer';
      deleteButton.addEventListener('click', () => {
        void deleteItem(item);
      });

      actionsNode.appendChild(playButton);
      actionsNode.appendChild(bindButton);
      actionsNode.appendChild(clearButton);
      actionsNode.appendChild(deleteButton);

      card.appendChild(titleNode);
      card.appendChild(subNode);
      card.appendChild(shortcutNode);
      card.appendChild(actionsNode);

      itemsListNode.appendChild(card);
    }

    renderPreview();
  };

  const fetchItems = async () => {
    const endpoint = buildAuthedUrl('/overlay/meme-board/items', {
      limit: 150,
      offset: 0,
      q: state.search,
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

    state.items = Array.isArray(payload?.items) ? payload.items : [];
    state.total = typeof payload?.total === 'number' ? payload.total : state.items.length;

    if (!state.selectedId || !state.items.find((item) => item.id === state.selectedId)) {
      state.selectedId = state.items[0]?.id || null;
    }
  };

  const loadItemsAndRender = async () => {
    try {
      await fetchItems();
      renderList();
      clearStatus();
    } catch (error) {
      setStatus(error?.message || 'Erreur de chargement de la meme board.', 'error');
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
      const result = await window.livechatOverlay.triggerMeme({
        itemId,
        trigger: 'ui',
      });

      if (!result?.ok) {
        throw new Error(result?.reason || 'socket_offline');
      }

      setStatus('Meme envoye dans la file.', 'success');
    } catch (error) {
      setStatus(`Echec trigger: ${error?.message || error}`, 'error');
    }
  };

  const clearShortcutForItem = async (itemId) => {
    const nextBindings = { ...state.bindings };

    for (const [accelerator, mappedItemId] of Object.entries(nextBindings)) {
      if (mappedItemId === itemId) {
        delete nextBindings[accelerator];
      }
    }

    try {
      await persistBindings(nextBindings);
      renderList();
      setStatus('Raccourci retire.', 'success');
    } catch (error) {
      setStatus(error?.message || 'Impossible de retirer le raccourci.', 'error');
    }
  };

  const deleteItem = async (item) => {
    const confirmed = window.confirm(`Supprimer "${toCardTitle(item)}" de la meme board ?`);

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
      }

      await loadItemsAndRender();
      setStatus('Meme supprime.', 'success');
    } catch (error) {
      setStatus(`Suppression impossible: ${error?.message || error}`, 'error');
    }
  };

  const beginCaptureForItem = (itemId) => {
    state.captureItemId = itemId;
    captureOverlay.classList.remove('hidden');
    setStatus('Capture clavier active: appuie sur un raccourci (Esc pour annuler).', 'success');
  };

  const endCapture = () => {
    state.captureItemId = null;
    captureOverlay.classList.add('hidden');
  };

  const normalizeKey = (eventKey) => {
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
    const key = normalizeKey(event.key);

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

  window.addEventListener('keydown', (event) => {
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

    if (!accelerator) {
      setStatus('Combinaison invalide. Utilise au moins un modificateur (Ctrl/Alt/Shift/Super).', 'error');
      return;
    }

    void (async () => {
      const itemId = state.captureItemId;
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
        setStatus(`Raccourci assigne: ${accelerator}`, 'success');
      } catch (error) {
        setStatus(error?.message || 'Impossible d assigner ce raccourci.', 'error');
      }
    })();
  });

  searchForm.addEventListener('submit', (event) => {
    event.preventDefault();

    state.search = `${searchInput.value || ''}`.trim();
    void loadItemsAndRender();
  });

  refreshButton.addEventListener('click', () => {
    void loadItemsAndRender();
  });

  const bootstrap = async () => {
    state.config = await window.livechatOverlay.getConfig();

    if (!state.config?.serverUrl || !state.config?.clientToken || !state.config?.guildId) {
      setStatus('Overlay non appaire. Ouvre la fenetre d appairage puis reconnecte.', 'error');
      refreshButton.disabled = true;
      searchInput.disabled = true;
      return;
    }

    const bindingsResult = await window.livechatOverlay.getMemeBindings();
    state.bindings = bindingsResult?.bindings || {};

    await loadItemsAndRender();
  };

  bootstrap().catch((error) => {
    setStatus(error?.message || 'Initialisation meme board impossible.', 'error');
  });
})();
