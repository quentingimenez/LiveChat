const { app, BrowserWindow, screen, Tray, Menu, nativeImage, globalShortcut, ipcMain } = require('electron');
const path = require('path');
const fs = require('fs');
const http = require('http');
const https = require('https');
const { io } = require('socket.io-client');
const { OVERLAY_SOCKET_EVENTS } = require('./protocol');

app.commandLine.appendSwitch('ignore-certificate-errors');
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');
if (process.platform === 'win32') {
  app.setAppUserModelId('com.overlay.client');
}

let overlayWindow;
let pairingWindow;
let boardWindow;
let tray;
let overlaySocket;
let keepOnTopInterval;
let heartbeatInterval;
let overlayConnectionState = 'disconnected';
let overlayConnectionReason = '';
let pendingPlaybackStatePayload = null;
let pendingPlaybackStopPayload = null;
let activeMemeBindings = {};

const configPath = path.join(app.getPath('userData'), 'config.json');
const appIconPath = path.join(__dirname, 'icon.png');

const defaultConfig = {
  displayId: null,
  displayIndex: null,
  volume: 1.0,
  enabled: true,
  autoStart: false,
  showText: true,
  serverUrl: null,
  clientToken: null,
  clientId: null,
  guildId: null,
  deviceName: null,
  memeBindings: {},
};

const MANUAL_RELOAD_SHORTCUT = 'Shift+Escape';

const CONNECTION_STATE_LABELS = {
  disabled: 'Désactivé',
  not_paired: 'Non appairé',
  connecting: 'Connexion...',
  reconnecting: 'Reconnexion...',
  connected: 'Connecté',
  disconnected: 'Déconnecté',
  error: 'Erreur',
};

const VOLUME_PRESETS = Object.freeze([
  { label: 'Muet (0%)', value: 0 },
  { label: '10%', value: 0.1 },
  { label: '15%', value: 0.15 },
  { label: '20%', value: 0.2 },
  { label: '25%', value: 0.25 },
  { label: '35%', value: 0.35 },
  { label: '50%', value: 0.5 },
  { label: '65%', value: 0.65 },
  { label: '75%', value: 0.75 },
  { label: '85%', value: 0.85 },
  { label: '100%', value: 1 },
]);

function normalizeVolume(volume) {
  if (typeof volume !== 'number' || !Number.isFinite(volume)) {
    return defaultConfig.volume;
  }

  return Math.min(1, Math.max(0, volume));
}

function normalizeAutoStart(value) {
  return value === true;
}

function normalizeMemeBindings(candidate) {
  if (!candidate || typeof candidate !== 'object' || Array.isArray(candidate)) {
    return {};
  }

  const normalized = {};

  for (const [rawAccelerator, rawItemId] of Object.entries(candidate)) {
    const accelerator = `${rawAccelerator || ''}`.trim();
    const itemId = `${rawItemId || ''}`.trim();

    if (!accelerator || !itemId) {
      continue;
    }

    normalized[accelerator] = itemId;
  }

  return normalized;
}

function getClosestVolumePreset(volume) {
  const normalizedVolume = normalizeVolume(volume);
  let selectedPreset = VOLUME_PRESETS[0];

  for (const preset of VOLUME_PRESETS) {
    if (Math.abs(preset.value - normalizedVolume) < Math.abs(selectedPreset.value - normalizedVolume)) {
      selectedPreset = preset;
    }
  }

  return selectedPreset;
}

function buildVolumeMenuItems(currentVolume) {
  const selectedPreset = getClosestVolumePreset(currentVolume);

  return VOLUME_PRESETS.map((preset) => ({
    label: preset.label,
    type: 'radio',
    checked: preset.value === selectedPreset.value,
    click: () => changeVolume(preset.value),
  }));
}

function loadConfig() {
  try {
    if (!fs.existsSync(configPath)) {
      return { ...defaultConfig };
    }

    const parsed = JSON.parse(fs.readFileSync(configPath, 'utf-8'));

    return {
      ...defaultConfig,
      ...parsed,
      volume: normalizeVolume(parsed.volume),
      autoStart: normalizeAutoStart(parsed.autoStart),
      memeBindings: normalizeMemeBindings(parsed.memeBindings),
    };
  } catch (error) {
    console.error('Unable to read config:', error);
    return { ...defaultConfig };
  }
}

function saveConfig(nextValues) {
  try {
    const merged = {
      ...loadConfig(),
      ...nextValues,
    };

    const normalizedMerged = {
      ...merged,
      volume: normalizeVolume(merged.volume),
      autoStart: normalizeAutoStart(merged.autoStart),
      memeBindings: normalizeMemeBindings(merged.memeBindings),
    };

    fs.writeFileSync(configPath, JSON.stringify(normalizedMerged, null, 2));
    return normalizedMerged;
  } catch (error) {
    console.error('Unable to save config:', error);
    return loadConfig();
  }
}

function hasPairingConfig(config = loadConfig()) {
  return !!(config.serverUrl && config.clientToken && config.guildId);
}

function getConnectionStateLabel() {
  return CONNECTION_STATE_LABELS[overlayConnectionState] || overlayConnectionState;
}

function setOverlayConnectionState(nextState, reason = '') {
  overlayConnectionState = nextState;
  overlayConnectionReason = typeof reason === 'string' ? reason.trim() : '';

  if (tray) {
    const cfg = loadConfig();
    const suffix = cfg.deviceName ? ` (${cfg.deviceName})` : '';
    tray.setToolTip(`Overlay ${getConnectionStateLabel()}${suffix}`);
  }

  updateTrayMenu();
}

function normalizeServerUrl(serverUrl) {
  return serverUrl.trim().replace(/\/+$/, '');
}

function supportsAutoStart() {
  if (typeof app.getLoginItemSettings !== 'function' || typeof app.setLoginItemSettings !== 'function') {
    return false;
  }

  return process.platform === 'win32' || process.platform === 'darwin';
}

function getSystemAutoStartEnabled() {
  if (!supportsAutoStart()) {
    return false;
  }

  try {
    return app.getLoginItemSettings().openAtLogin === true;
  } catch (error) {
    console.error('Unable to read auto-start setting:', error);
    return false;
  }
}

function applyAutoStartSetting(enabled) {
  const nextEnabled = enabled === true;

  if (!supportsAutoStart()) {
    saveConfig({ autoStart: false });
    return false;
  }

  try {
    const loginItemSettings = {
      openAtLogin: nextEnabled,
    };

    if (process.platform === 'darwin') {
      loginItemSettings.openAsHidden = true;
    }

    if (process.platform === 'win32') {
      loginItemSettings.path = process.execPath;
      loginItemSettings.args = [];
    }

    app.setLoginItemSettings(loginItemSettings);
  } catch (error) {
    console.error('Unable to update auto-start setting:', error);
  }

  const systemEnabled = getSystemAutoStartEnabled();
  saveConfig({ autoStart: systemEnabled });
  updateTrayMenu();
  return systemEnabled;
}

const TLS_ERROR_CODES = new Set([
  'DEPTH_ZERO_SELF_SIGNED_CERT',
  'UNABLE_TO_VERIFY_LEAF_SIGNATURE',
  'SELF_SIGNED_CERT_IN_CHAIN',
  'CERT_HAS_EXPIRED',
  'ERR_TLS_CERT_ALTNAME_INVALID',
]);

function isLikelyTlsError(error) {
  if (!error) {
    return false;
  }

  const code = `${error.code || ''}`.toUpperCase();
  const message = `${error.message || ''}`.toLowerCase();

  return TLS_ERROR_CODES.has(code) || message.includes('certificate') || message.includes('tls');
}

function formatNetworkError(error, endpoint) {
  const code = `${error?.code || ''}`.toUpperCase();
  const message = error?.message || 'unknown network error';

  if (code === 'ENOTFOUND') {
    return `dns_unreachable (${endpoint})`;
  }

  if (code === 'ECONNREFUSED') {
    return `connection_refused (${endpoint})`;
  }

  if (code === 'ETIMEDOUT' || code === 'ECONNABORTED') {
    return `request_timeout (${endpoint})`;
  }

  if (isLikelyTlsError(error)) {
    return `tls_error (${message})`;
  }

  return `network_error (${code || 'UNKNOWN'}: ${message})`;
}

function httpRequestJson(url, payload, options = {}) {
  const target = new URL(url);
  const isHttps = target.protocol === 'https:';
  const client = isHttps ? https : http;
  const requestBody = JSON.stringify(payload);

  return new Promise((resolve, reject) => {
    const req = client.request(
      {
        protocol: target.protocol,
        hostname: target.hostname,
        port: target.port || (isHttps ? 443 : 80),
        path: `${target.pathname}${target.search}`,
        method: 'POST',
        rejectUnauthorized: options.rejectUnauthorized !== false,
        timeout: options.timeoutMs || 10000,
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(requestBody),
        },
      },
      (res) => {
        let responseBody = '';

        res.setEncoding('utf8');
        res.on('data', (chunk) => {
          responseBody += chunk;
        });
        res.on('end', () => {
          resolve({
            statusCode: res.statusCode || 0,
            body: responseBody,
          });
        });
      },
    );

    req.on('timeout', () => {
      req.destroy(Object.assign(new Error('request_timeout'), { code: 'ETIMEDOUT' }));
    });

    req.on('error', (error) => {
      reject(error);
    });

    req.write(requestBody);
    req.end();
  });
}

function getTargetDisplay() {
  const displays = screen.getAllDisplays();
  const cfg = loadConfig();

  if (cfg.displayId !== null) {
    const byId = displays.find((display) => display.id === cfg.displayId);
    if (byId) {
      return byId;
    }
  }

  if (Number.isInteger(cfg.displayIndex) && displays[cfg.displayIndex]) {
    return displays[cfg.displayIndex];
  }

  return screen.getPrimaryDisplay();
}

function startKeepOnTopLoop() {
  stopKeepOnTopLoop();

  keepOnTopInterval = setInterval(() => {
    if (overlayWindow && !overlayWindow.isDestroyed()) {
      overlayWindow.setAlwaysOnTop(true, 'screen-saver');
    }
  }, 2000);
}

function stopKeepOnTopLoop() {
  if (keepOnTopInterval) {
    clearInterval(keepOnTopInterval);
    keepOnTopInterval = null;
  }
}

function sendOverlaySettingsToRenderer() {
  if (!overlayWindow || overlayWindow.isDestroyed()) {
    return;
  }

  const cfg = loadConfig();

  overlayWindow.webContents.send('overlay:settings', {
    volume: cfg.volume,
    showText: cfg.showText,
  });
}

function createOverlayWindow() {
  if (overlayWindow && !overlayWindow.isDestroyed()) {
    return;
  }

  const targetDisplay = getTargetDisplay();
  const displays = screen.getAllDisplays();
  const displayIndex = displays.findIndex((display) => display.id === targetDisplay.id);

  saveConfig({
    displayId: targetDisplay.id,
    displayIndex,
  });

  const { width, height, x, y } = targetDisplay.bounds;

  overlayWindow = new BrowserWindow({
    width,
    height,
    x,
    y,
    transparent: true,
    frame: false,
    alwaysOnTop: true,
    focusable: false,
    hasShadow: false,
    skipTaskbar: true,
    icon: appIconPath,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js'),
      backgroundThrottling: false,
    },
  });

  overlayWindow.loadFile(path.join(__dirname, 'renderer/overlay.html'));
  overlayWindow.setIgnoreMouseEvents(true, { forward: true });

  overlayWindow.once('ready-to-show', () => {
    overlayWindow.show();
    sendOverlaySettingsToRenderer();
  });

  overlayWindow.on('closed', () => {
    overlayWindow = null;
    stopKeepOnTopLoop();
    updateTrayMenu();
  });

  startKeepOnTopLoop();
}

function destroyOverlayWindow() {
  stopKeepOnTopLoop();

  if (overlayWindow && !overlayWindow.isDestroyed()) {
    overlayWindow.destroy();
  }

  overlayWindow = null;
}

function createPairingWindow() {
  if (pairingWindow && !pairingWindow.isDestroyed()) {
    pairingWindow.focus();
    return;
  }

  pairingWindow = new BrowserWindow({
    width: 460,
    height: 540,
    resizable: false,
    autoHideMenuBar: true,
    icon: appIconPath,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js'),
    },
  });

  pairingWindow.loadFile(path.join(__dirname, 'renderer/pairing.html'));

  pairingWindow.on('closed', () => {
    pairingWindow = null;
  });
}

function closePairingWindow() {
  if (pairingWindow && !pairingWindow.isDestroyed()) {
    pairingWindow.close();
  }
}

function createBoardWindow() {
  if (!hasPairingConfig()) {
    createPairingWindow();
    return;
  }

  if (boardWindow && !boardWindow.isDestroyed()) {
    boardWindow.focus();
    return;
  }

  boardWindow = new BrowserWindow({
    width: 1180,
    height: 780,
    minWidth: 960,
    minHeight: 640,
    autoHideMenuBar: true,
    icon: appIconPath,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      preload: path.join(__dirname, 'preload.js'),
    },
  });

  boardWindow.loadFile(path.join(__dirname, 'renderer/board.html'));

  boardWindow.on('closed', () => {
    boardWindow = null;
  });
}

function destroyBoardWindow() {
  if (boardWindow && !boardWindow.isDestroyed()) {
    boardWindow.destroy();
  }

  boardWindow = null;
}

function stopHeartbeatLoop() {
  if (heartbeatInterval) {
    clearInterval(heartbeatInterval);
    heartbeatInterval = null;
  }
}

function startHeartbeatLoop() {
  stopHeartbeatLoop();

  heartbeatInterval = setInterval(() => {
    const cfg = loadConfig();

    if (!overlaySocket || !overlaySocket.connected) {
      return;
    }

    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.HEARTBEAT, {
      clientId: cfg.clientId || 'unknown-client',
      guildId: cfg.guildId || 'unknown-guild',
      appVersion: app.getVersion(),
    });
  }, 15000);
}

function disconnectOverlaySocket(options = {}) {
  const nextState = options.nextState || 'disconnected';
  const reason = options.reason || '';
  const keepStatus = options.keepStatus === true;

  stopHeartbeatLoop();

  if (overlaySocket) {
    overlaySocket.disconnect();
    overlaySocket = null;
  }

  if (!keepStatus) {
    setOverlayConnectionState(nextState, reason);
  }
}

function connectOverlaySocket() {
  const cfg = loadConfig();

  disconnectOverlaySocket({ keepStatus: true });

  if (!cfg.enabled || !hasPairingConfig(cfg)) {
    setOverlayConnectionState(cfg.enabled ? 'not_paired' : 'disabled');
    return;
  }

  setOverlayConnectionState('connecting');

  overlaySocket = io(cfg.serverUrl, {
    transports: ['websocket'],
    reconnection: true,
    reconnectionDelay: 1000,
    reconnectionDelayMax: 3000,
    timeout: 10000,
    rejectUnauthorized: false,
    auth: {
      token: cfg.clientToken,
    },
  });

  overlaySocket.on('connect', () => {
    startHeartbeatLoop();
    setOverlayConnectionState('connected');
    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.HEARTBEAT, {
      clientId: cfg.clientId || 'unknown-client',
      guildId: cfg.guildId || 'unknown-guild',
      appVersion: app.getVersion(),
    });

    if (pendingPlaybackStatePayload) {
      overlaySocket.emit(OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE, pendingPlaybackStatePayload);
      console.info(
        `[OVERLAY] Flushed buffered playback-state (jobId: ${
          pendingPlaybackStatePayload.jobId || 'unknown'
        }, state: ${pendingPlaybackStatePayload.state || 'unknown'})`,
      );
      pendingPlaybackStatePayload = null;
    }

    if (pendingPlaybackStopPayload) {
      overlaySocket.emit(OVERLAY_SOCKET_EVENTS.STOP, pendingPlaybackStopPayload);
      console.info(
        `[OVERLAY] Flushed buffered playback-stop (jobId: ${pendingPlaybackStopPayload.jobId || 'unknown'})`,
      );
      pendingPlaybackStopPayload = null;
    }
  });

  overlaySocket.on(OVERLAY_SOCKET_EVENTS.PLAY, (payload) => {
    if (!overlayWindow || overlayWindow.isDestroyed()) {
      return;
    }

    overlayWindow.webContents.send('overlay:play', payload);
  });

  overlaySocket.on(OVERLAY_SOCKET_EVENTS.STOP, (payload) => {
    if (!overlayWindow || overlayWindow.isDestroyed()) {
      return;
    }

    overlayWindow.webContents.send('overlay:stop', payload);
  });

  overlaySocket.on('disconnect', (reason) => {
    stopHeartbeatLoop();
    const current = loadConfig();

    if (!current.enabled) {
      setOverlayConnectionState('disabled', reason);
      return;
    }

    if (!hasPairingConfig(current)) {
      setOverlayConnectionState('not_paired', reason);
      return;
    }

    if (reason === 'io client disconnect') {
      setOverlayConnectionState('disconnected', reason);
      return;
    }

    setOverlayConnectionState('reconnecting', reason);
  });

  overlaySocket.on('connect_error', (error) => {
    console.error('Overlay socket connection failed:', error?.message || error);
    setOverlayConnectionState('error', error?.message || 'connect_error');
  });

  if (overlaySocket.io) {
    overlaySocket.io.on('reconnect_attempt', () => {
      setOverlayConnectionState('reconnecting');
    });
  }
}

async function setEnabledAsync(enabled) {
  saveConfig({ enabled });

  if (!enabled) {
    disconnectOverlaySocket({ nextState: 'disabled' });
    destroyOverlayWindow();
    destroyBoardWindow();
    return;
  }

  if (!hasPairingConfig()) {
    setOverlayConnectionState('not_paired');
    createPairingWindow();
    return;
  }

  createOverlayWindow();
  connectOverlaySocket();
}

function changeVolume(level) {
  saveConfig({ volume: normalizeVolume(level) });
  sendOverlaySettingsToRenderer();
  updateTrayMenu();
}

function toggleShowText(checked) {
  saveConfig({ showText: checked });
  sendOverlaySettingsToRenderer();
  updateTrayMenu();
}

function toggleAutoStart(checked) {
  applyAutoStartSetting(checked);
}

function emitManualStopSignal() {
  if (!overlaySocket || !overlaySocket.connected) {
    return;
  }

  overlaySocket.emit(OVERLAY_SOCKET_EVENTS.STOP, {
    jobId: 'manual-stop',
  });
}

function emitMemeTriggerSignal(itemId, trigger = 'shortcut') {
  const normalizedItemId = `${itemId || ''}`.trim();
  const normalizedTrigger = trigger === 'ui' ? 'ui' : 'shortcut';

  if (!normalizedItemId) {
    return {
      ok: false,
      reason: 'invalid_item_id',
    };
  }

  if (!overlaySocket || !overlaySocket.connected) {
    return {
      ok: false,
      reason: 'socket_offline',
    };
  }

  overlaySocket.emit(OVERLAY_SOCKET_EVENTS.MEME_TRIGGER, {
    itemId: normalizedItemId,
    trigger: normalizedTrigger,
  });
  console.info(`[OVERLAY] Forwarded meme trigger to bot (itemId: ${normalizedItemId}, trigger: ${normalizedTrigger})`);

  return {
    ok: true,
  };
}

function unregisterMemeShortcuts() {
  for (const accelerator of Object.keys(activeMemeBindings)) {
    globalShortcut.unregister(accelerator);
  }

  activeMemeBindings = {};
}

function tryRegisterMemeBindings(bindings) {
  const normalizedBindings = normalizeMemeBindings(bindings);
  const failures = [];
  const registered = {};

  for (const [accelerator, itemId] of Object.entries(normalizedBindings)) {
    const registeredOk = globalShortcut.register(accelerator, () => {
      emitMemeTriggerSignal(itemId, 'shortcut');
    });

    if (!registeredOk) {
      failures.push(accelerator);
      continue;
    }

    registered[accelerator] = itemId;
  }

  return {
    registered,
    failures,
  };
}

function applyMemeBindings(nextBindings, options = {}) {
  const strict = options.strict !== false;
  const persist = options.persist !== false;
  const previousBindings = { ...activeMemeBindings };

  unregisterMemeShortcuts();
  const registrationResult = tryRegisterMemeBindings(nextBindings);

  if (strict && registrationResult.failures.length > 0) {
    unregisterMemeShortcuts();
    const rollbackResult = tryRegisterMemeBindings(previousBindings);
    activeMemeBindings = rollbackResult.registered;

    return {
      ok: false,
      appliedBindings: { ...activeMemeBindings },
      failedAccelerators: registrationResult.failures,
    };
  }

  activeMemeBindings = registrationResult.registered;

  if (persist) {
    saveConfig({
      memeBindings: activeMemeBindings,
    });
  }

  return {
    ok: registrationResult.failures.length === 0,
    appliedBindings: { ...activeMemeBindings },
    failedAccelerators: registrationResult.failures,
  };
}

function moveOverlayToDisplay(display, index) {
  saveConfig({
    displayId: display.id,
    displayIndex: index,
  });

  if (overlayWindow && !overlayWindow.isDestroyed()) {
    overlayWindow.setBounds({
      x: display.bounds.x,
      y: display.bounds.y,
      width: display.bounds.width,
      height: display.bounds.height,
    });
  }

  updateTrayMenu();
}

function resetPairing() {
  saveConfig({
    serverUrl: null,
    clientToken: null,
    guildId: null,
    clientId: null,
    deviceName: null,
  });

  disconnectOverlaySocket({ nextState: 'not_paired' });
  destroyOverlayWindow();
  destroyBoardWindow();

  if (loadConfig().enabled) {
    createPairingWindow();
  }
}

function updateTrayMenu() {
  if (!tray) {
    return;
  }

  const cfg = loadConfig();
  const displays = screen.getAllDisplays();
  const autoStartSupported = supportsAutoStart();
  const suffix = cfg.deviceName ? ` (${cfg.deviceName})` : '';
  tray.setToolTip(`Overlay ${getConnectionStateLabel()}${suffix}`);

  const template = [
    {
      label: `Statut connexion: ${getConnectionStateLabel()}`,
      enabled: false,
    },
    ...(overlayConnectionReason
      ? [
          {
            label: `Raison: ${overlayConnectionReason}`,
            enabled: false,
          },
        ]
      : []),
    { type: 'separator' },
    {
      label: 'Overlay activé',
      type: 'checkbox',
      checked: cfg.enabled,
      click: ({ checked }) => setEnabledAsync(checked),
    },
    {
      label: autoStartSupported ? 'Lancer au démarrage' : 'Lancer au démarrage (non supporté)',
      type: 'checkbox',
      checked: autoStartSupported && cfg.autoStart === true,
      enabled: autoStartSupported,
      click: ({ checked }) => toggleAutoStart(checked),
    },
    {
      label: 'Afficher le texte',
      type: 'checkbox',
      checked: cfg.showText,
      click: ({ checked }) => toggleShowText(checked),
    },
    { type: 'separator' },
    {
      label: 'Appairer Overlay',
      click: () => createPairingWindow(),
    },
    {
      label: 'Ouvrir Meme Board',
      enabled: cfg.enabled && hasPairingConfig(cfg),
      click: () => createBoardWindow(),
    },
    { type: 'separator' },
    { label: 'CONFIGURATION ÉCRAN', enabled: false },
    { type: 'separator' },
  ];

  displays.forEach((display, index) => {
    template.push({
      label: `Écran ${index + 1} (${display.bounds.width}x${display.bounds.height})`,
      type: 'radio',
      checked:
        (cfg.displayId !== null && display.id === cfg.displayId) ||
        (cfg.displayId === null && Number.isInteger(cfg.displayIndex) && cfg.displayIndex === index),
      click: () => moveOverlayToDisplay(display, index),
    });
  });

  template.push({ type: 'separator' });

  template.push({
    label: 'Volume Audio',
    submenu: buildVolumeMenuItems(cfg.volume),
  });

  template.push({ type: 'separator' });

  template.push({
    label: 'Quitter',
    click: () => {
      disconnectOverlaySocket();
      destroyOverlayWindow();
      destroyBoardWindow();
      closePairingWindow();
      app.quit();
    },
  });

  tray.setContextMenu(Menu.buildFromTemplate(template));
}

function createTray() {
  const icon = nativeImage.createFromPath(appIconPath);
  tray = new Tray(icon);

  updateTrayMenu();
  tray.on('double-click', () => {
    createBoardWindow();
  });

  screen.on('display-added', updateTrayMenu);
  screen.on('display-removed', updateTrayMenu);
  screen.on('display-metrics-changed', updateTrayMenu);
}

function registerShortcuts() {
  globalShortcut.register(MANUAL_RELOAD_SHORTCUT, () => {
    const cfg = loadConfig();

    if (cfg.enabled && overlayWindow && !overlayWindow.isDestroyed()) {
      emitManualStopSignal();
      overlayWindow.reload();
    }
  });

  applyMemeBindings(loadConfig().memeBindings, {
    strict: false,
    persist: false,
  });
}

ipcMain.handle('overlay:get-config', () => {
  const cfg = loadConfig();

  return {
    serverUrl: cfg.serverUrl,
    clientToken: cfg.clientToken,
    guildId: cfg.guildId,
    clientId: cfg.clientId,
    showText: cfg.showText,
    volume: cfg.volume,
    memeBindings: normalizeMemeBindings(cfg.memeBindings),
  };
});

ipcMain.handle('meme-board:get-bindings', () => {
  const cfg = loadConfig();

  return {
    bindings: normalizeMemeBindings(cfg.memeBindings),
  };
});

ipcMain.handle('meme-board:set-bindings', (_event, payload) => {
  const nextBindings = normalizeMemeBindings(payload?.bindings);
  const applyResult = applyMemeBindings(nextBindings, {
    strict: true,
    persist: true,
  });

  return {
    ok: applyResult.ok,
    bindings: applyResult.appliedBindings,
    failedAccelerators: applyResult.failedAccelerators,
  };
});

ipcMain.handle('meme-board:trigger', (_event, payload) => {
  return emitMemeTriggerSignal(payload?.itemId, payload?.trigger);
});

ipcMain.on('overlay:renderer-ready', () => {
  sendOverlaySettingsToRenderer();
});

ipcMain.on('overlay:error', (_event, payload) => {
  if (!overlaySocket || !overlaySocket.connected) {
    return;
  }

  overlaySocket.emit(OVERLAY_SOCKET_EVENTS.ERROR, payload);
});

ipcMain.on('overlay:playback-state', (_event, payload) => {
  if (!overlaySocket || !overlaySocket.connected) {
    pendingPlaybackStatePayload = payload || null;
    console.warn(
      `[OVERLAY] Buffered playback-state while socket offline (jobId: ${
        payload?.jobId || 'unknown'
      }, state: ${payload?.state || 'unknown'})`,
    );
    return;
  }

  overlaySocket.emit(OVERLAY_SOCKET_EVENTS.PLAYBACK_STATE, payload);
  console.info(
    `[OVERLAY] Forwarded playback-state to bot (jobId: ${payload?.jobId || 'unknown'}, state: ${
      payload?.state || 'unknown'
    }, remainingMs: ${typeof payload?.remainingMs === 'number' ? payload.remainingMs : 'null'})`,
  );
});

ipcMain.on('overlay:playback-stop', (_event, payload) => {
  const normalizedPayload = {
    jobId: typeof payload?.jobId === 'string' && payload.jobId.trim() ? payload.jobId.trim() : 'unknown',
  };

  if (!overlaySocket || !overlaySocket.connected) {
    pendingPlaybackStopPayload = normalizedPayload;
    console.warn(`[OVERLAY] Buffered playback-stop while socket offline (jobId: ${normalizedPayload.jobId})`);
    return;
  }

  overlaySocket.emit(OVERLAY_SOCKET_EVENTS.STOP, normalizedPayload);
  console.info(`[OVERLAY] Forwarded playback-stop to bot (jobId: ${normalizedPayload.jobId})`);
});

ipcMain.handle('pairing:consume', async (_event, payload) => {
  const serverUrl = normalizeServerUrl(`${payload?.serverUrl || ''}`);
  const code = `${payload?.code || ''}`.toUpperCase().trim();
  const requestedDeviceName = `${payload?.deviceName || ''}`.trim();

  if (!serverUrl || !code) {
    throw new Error('missing_required_fields');
  }

  const endpoint = `${serverUrl}/overlay/pair/consume`;
  let pairingResponse;

  try {
    pairingResponse = await httpRequestJson(
      endpoint,
      {
        code,
        deviceName: requestedDeviceName || undefined,
      },
      {
        rejectUnauthorized: true,
      },
    );
  } catch (error) {
    if (endpoint.startsWith('https://') && isLikelyTlsError(error)) {
      pairingResponse = await httpRequestJson(
        endpoint,
        {
          code,
          deviceName: requestedDeviceName || undefined,
        },
        {
          rejectUnauthorized: false,
        },
      );
    } else {
      throw new Error(formatNetworkError(error, endpoint));
    }
  }

  const payloadText = pairingResponse.body || '';

  if (pairingResponse.statusCode < 200 || pairingResponse.statusCode >= 300) {
    throw new Error(payloadText || `pairing_failed_${pairingResponse.statusCode}`);
  }

  let parsed;

  try {
    parsed = JSON.parse(payloadText);
  } catch {
    throw new Error('invalid_pairing_response');
  }

  const resolvedDeviceName = `${parsed?.deviceName || requestedDeviceName || ''}`.trim() || null;

  saveConfig({
    serverUrl: normalizeServerUrl(parsed.apiBaseUrl || serverUrl),
    clientToken: parsed.clientToken,
    clientId: parsed.clientId,
    guildId: parsed.guildId,
    deviceName: resolvedDeviceName,
  });

  if (loadConfig().enabled) {
    createOverlayWindow();
    connectOverlaySocket();
  }

  closePairingWindow();
  updateTrayMenu();

  return {
    ok: true,
  };
});

app.whenReady().then(async () => {
  const cfgBeforeAutoStart = loadConfig();
  let hasPersistedAutoStart = false;

  try {
    if (fs.existsSync(configPath)) {
      const rawConfig = JSON.parse(fs.readFileSync(configPath, 'utf-8'));
      hasPersistedAutoStart = typeof rawConfig?.autoStart === 'boolean';
    }
  } catch (error) {
    console.error('Unable to inspect persisted auto-start preference:', error);
  }

  if (supportsAutoStart()) {
    if (hasPersistedAutoStart) {
      applyAutoStartSetting(cfgBeforeAutoStart.autoStart === true);
    } else {
      saveConfig({ autoStart: getSystemAutoStartEnabled() });
    }
  } else if (cfgBeforeAutoStart.autoStart) {
    saveConfig({ autoStart: false });
  }

  createTray();
  registerShortcuts();

  const cfg = loadConfig();

  if (!cfg.enabled) {
    setOverlayConnectionState('disabled');
  } else if (!hasPairingConfig(cfg)) {
    setOverlayConnectionState('not_paired');
  } else {
    setOverlayConnectionState('connecting');
  }

  if (cfg.enabled) {
    if (hasPairingConfig(cfg)) {
      createOverlayWindow();
      connectOverlaySocket();
    } else {
      createPairingWindow();
    }
  }

  app.on('activate', () => {
    const current = loadConfig();

    if (!current.enabled) {
      return;
    }

    if (!hasPairingConfig(current)) {
      createPairingWindow();
      return;
    }

    if (!overlayWindow) {
      createOverlayWindow();
      connectOverlaySocket();
    }
  });
});

app.on('window-all-closed', () => {
  // Keep app running in tray.
});

app.on('will-quit', () => {
  disconnectOverlaySocket();
  stopKeepOnTopLoop();
  destroyBoardWindow();
  globalShortcut.unregisterAll();
});
