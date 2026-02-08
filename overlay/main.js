const { app, BrowserWindow, screen, Tray, Menu, nativeImage, globalShortcut, ipcMain } = require('electron');
const path = require('path');
const fs = require('fs');
const http = require('http');
const https = require('https');
const { io } = require('socket.io-client');
const { OVERLAY_SOCKET_EVENTS } = require('./protocol');

app.commandLine.appendSwitch('ignore-certificate-errors');
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');

let overlayWindow;
let pairingWindow;
let tray;
let overlaySocket;
let keepOnTopInterval;
let heartbeatInterval;

const configPath = path.join(app.getPath('userData'), 'config.json');

const defaultConfig = {
  displayId: null,
  displayIndex: null,
  volume: 1.0,
  enabled: true,
  showText: true,
  serverUrl: null,
  clientToken: null,
  clientId: null,
  guildId: null,
};

function loadConfig() {
  try {
    if (!fs.existsSync(configPath)) {
      return { ...defaultConfig };
    }

    const parsed = JSON.parse(fs.readFileSync(configPath, 'utf-8'));

    return {
      ...defaultConfig,
      ...parsed,
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

    fs.writeFileSync(configPath, JSON.stringify(merged, null, 2));
    return merged;
  } catch (error) {
    console.error('Unable to save config:', error);
    return loadConfig();
  }
}

function hasPairingConfig(config = loadConfig()) {
  return !!(config.serverUrl && config.clientToken && config.guildId);
}

function normalizeServerUrl(serverUrl) {
  return serverUrl.trim().replace(/\/+$/, '');
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

function disconnectOverlaySocket() {
  stopHeartbeatLoop();

  if (overlaySocket) {
    overlaySocket.disconnect();
    overlaySocket = null;
  }
}

function connectOverlaySocket() {
  const cfg = loadConfig();

  disconnectOverlaySocket();

  if (!cfg.enabled || !hasPairingConfig(cfg)) {
    return;
  }

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
    overlaySocket.emit(OVERLAY_SOCKET_EVENTS.HEARTBEAT, {
      clientId: cfg.clientId || 'unknown-client',
      guildId: cfg.guildId || 'unknown-guild',
      appVersion: app.getVersion(),
    });
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

  overlaySocket.on('disconnect', () => {
    stopHeartbeatLoop();
  });

  overlaySocket.on('connect_error', (error) => {
    console.error('Overlay socket connection failed:', error?.message || error);
  });
}

async function setEnabledAsync(enabled) {
  saveConfig({ enabled });

  if (!enabled) {
    disconnectOverlaySocket();
    destroyOverlayWindow();
    updateTrayMenu();
    return;
  }

  if (!hasPairingConfig()) {
    createPairingWindow();
    updateTrayMenu();
    return;
  }

  createOverlayWindow();
  connectOverlaySocket();
  updateTrayMenu();
}

function changeVolume(level) {
  saveConfig({ volume: level });
  sendOverlaySettingsToRenderer();
  updateTrayMenu();
}

function toggleShowText(checked) {
  saveConfig({ showText: checked });
  sendOverlaySettingsToRenderer();
  updateTrayMenu();
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
  });

  disconnectOverlaySocket();
  destroyOverlayWindow();

  if (loadConfig().enabled) {
    createPairingWindow();
  }

  updateTrayMenu();
}

function updateTrayMenu() {
  if (!tray) {
    return;
  }

  const cfg = loadConfig();
  const displays = screen.getAllDisplays();

  const template = [
    {
      label: 'Overlay activé',
      type: 'checkbox',
      checked: cfg.enabled,
      click: ({ checked }) => setEnabledAsync(checked),
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
      label: 'Réinitialiser appairage',
      click: () => resetPairing(),
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
    submenu: [
      { label: 'Muet (0%)', type: 'radio', checked: cfg.volume === 0, click: () => changeVolume(0) },
      { label: '25%', type: 'radio', checked: cfg.volume === 0.25, click: () => changeVolume(0.25) },
      { label: '50%', type: 'radio', checked: cfg.volume === 0.5, click: () => changeVolume(0.5) },
      { label: '75%', type: 'radio', checked: cfg.volume === 0.75, click: () => changeVolume(0.75) },
      { label: '100%', type: 'radio', checked: cfg.volume === 1, click: () => changeVolume(1) },
    ],
  });

  template.push({ type: 'separator' });

  template.push({
    label: 'Recharger (Shift+Echap)',
    enabled: cfg.enabled && !!overlayWindow && !overlayWindow.isDestroyed(),
    click: () => overlayWindow && overlayWindow.reload(),
  });

  template.push({
    label: 'Quitter',
    click: () => {
      disconnectOverlaySocket();
      destroyOverlayWindow();
      closePairingWindow();
      app.quit();
    },
  });

  tray.setContextMenu(Menu.buildFromTemplate(template));
}

function createTray() {
  const icon = nativeImage.createFromPath(path.join(__dirname, 'icon.png'));
  tray = new Tray(icon);
  tray.setToolTip('Overlay Audio/Video');

  updateTrayMenu();

  screen.on('display-added', updateTrayMenu);
  screen.on('display-removed', updateTrayMenu);
  screen.on('display-metrics-changed', updateTrayMenu);
}

function registerShortcuts() {
  globalShortcut.register('Shift+Escape', () => {
    const cfg = loadConfig();

    if (cfg.enabled && overlayWindow && !overlayWindow.isDestroyed()) {
      overlayWindow.reload();
    }
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
  };
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

ipcMain.handle('pairing:consume', async (_event, payload) => {
  const serverUrl = normalizeServerUrl(`${payload?.serverUrl || ''}`);
  const code = `${payload?.code || ''}`.toUpperCase().trim();
  const deviceName = `${payload?.deviceName || ''}`.trim();

  if (!serverUrl || !code || !deviceName) {
    throw new Error('missing_required_fields');
  }

  const endpoint = `${serverUrl}/overlay/pair/consume`;
  let pairingResponse;

  try {
    pairingResponse = await httpRequestJson(
      endpoint,
      {
        code,
        deviceName,
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
          deviceName,
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

  saveConfig({
    serverUrl: normalizeServerUrl(parsed.apiBaseUrl || serverUrl),
    clientToken: parsed.clientToken,
    clientId: parsed.clientId,
    guildId: parsed.guildId,
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
  createTray();
  registerShortcuts();

  const cfg = loadConfig();

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
  globalShortcut.unregisterAll();
});
