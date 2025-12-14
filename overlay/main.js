const { app, BrowserWindow, screen, Tray, Menu, nativeImage, globalShortcut } = require('electron');
const path = require('path');
const fs = require('fs');
const { execFile } = require('child_process');

// Options indispensables
app.commandLine.appendSwitch('ignore-certificate-errors');
app.commandLine.appendSwitch('autoplay-policy', 'no-user-gesture-required');
app.commandLine.appendSwitch('disable-site-isolation-trials');

let mainWindow;
let tray;
let keepOnTopInterval;

// Chemin vers le fichier de sauvegarde
const configPath = path.join(app.getPath('userData'), 'config.json');

// -------- CONFIG --------

function loadConfig() {
  try {
    if (fs.existsSync(configPath)) {
      const data = fs.readFileSync(configPath, 'utf-8');
      const p = JSON.parse(data);
      return {
        // Electron (fallbacks)
        displayId: p.displayId ?? null,
        displayKey: p.displayKey ?? null,
        displayIndex: p.displayIndex ?? null,

        // Windows (robuste)
        winMonitor: p.winMonitor ?? null, // { instanceName, modelCode, serial, friendlyName }

        // Audio
        volume: p.volume ?? 1.0,

        // ON/OFF
        enabled: p.enabled ?? true
      };
    }
  } catch (e) {
    console.error("Erreur lecture config:", e);
  }
  return {
    displayId: null,
    displayKey: null,
    displayIndex: null,
    winMonitor: null,
    volume: 1.0,
    enabled: true
  };
}

function saveConfig(newConfig) {
  try {
    const current = loadConfig();
    const toSave = { ...current, ...newConfig };
    fs.writeFileSync(configPath, JSON.stringify(toSave));
  } catch (e) {
    console.error("Erreur écriture config:", e);
  }
}

// Signature géométrique (fallback)
function makeDisplayKey(d) {
  const b = d.bounds;
  const sf = d.scaleFactor ?? 1;
  return `${b.x},${b.y},${b.width},${b.height},${sf}`;
}

function sameBounds(a, b) {
  return a && b &&
    a.x === b.x && a.y === b.y &&
    a.width === b.width && a.height === b.height;
}

// -------- WINDOWS: ECRANS + IDENTITE MONITEUR (WMI + EnumDisplayDevices) --------
//
// Retourne une liste de "screens" Windows avec bounds + (best effort) friendly/serial/instance.
// On matche ensuite avec Electron via bounds.
//
function getWinScreensDetailed() {
  return new Promise((resolve) => {
    if (process.platform !== 'win32') return resolve([]);

    const ps = `
Add-Type -AssemblyName System.Windows.Forms

# EnumDisplayDevices (WinAPI) pour lier \\\\.\\DISPLAYx à un DeviceID moniteur
$source = @"
using System;
using System.Runtime.InteropServices;

public class DisplayDevices {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Ansi)]
  public struct DISPLAY_DEVICE {
    public int cb;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
    public string DeviceName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
    public string DeviceString;
    public int StateFlags;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
    public string DeviceID;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
    public string DeviceKey;
  }

  [DllImport("user32.dll", CharSet = CharSet.Ansi)]
  public static extern bool EnumDisplayDevices(string lpDevice, uint iDevNum, ref DISPLAY_DEVICE lpDisplayDevice, uint dwFlags);

  public static DISPLAY_DEVICE GetDevice(string lpDevice, uint devNum) {
    DISPLAY_DEVICE d = new DISPLAY_DEVICE();
    d.cb = Marshal.SizeOf(d);
    EnumDisplayDevices(lpDevice, devNum, ref d, 0);
    return d;
  }
}
"@
Add-Type -TypeDefinition $source -ErrorAction SilentlyContinue | Out-Null

# 1) WMI: identifiants moniteurs (friendly + serial + instance)
$wmi = Get-CimInstance -Namespace root\\wmi -ClassName WmiMonitorID -ErrorAction SilentlyContinue |
  Where-Object { $_.Active -eq $true } |
  ForEach-Object {
    $friendly = ($_.UserFriendlyName | Where-Object { $_ -ne 0 } | ForEach-Object { [char]$_ }) -join ''
    $serial   = ($_.SerialNumberID | Where-Object { $_ -ne 0 } | ForEach-Object { [char]$_ }) -join ''
    $modelCode = ($_.InstanceName.Split('\\\\') | Select-Object -Skip 1 -First 1)
    [PSCustomObject]@{
      InstanceName = $_.InstanceName
      ModelCode = $modelCode
      FriendlyName = $friendly
      Serial = $serial
    }
  }

# Indexer WMI par modelCode (queue) pour associer en best-effort
$wmiByModel = @{}
foreach ($m in $wmi) {
  if (-not $wmiByModel.ContainsKey($m.ModelCode)) { $wmiByModel[$m.ModelCode] = New-Object System.Collections.ArrayList }
  [void]$wmiByModel[$m.ModelCode].Add($m)
}

# 2) Screens Windows (bounds + DeviceName)
$screens = [System.Windows.Forms.Screen]::AllScreens

# 3) Associer chaque Screen à un DeviceID moniteur via EnumDisplayDevices
$out = @()
foreach ($s in $screens) {
  $dn = $s.DeviceName  # ex: \\\\.\\DISPLAY1

  $disp = [DisplayDevices]::GetDevice($null, 0)
  $foundDisplay = $null
  for ($i=0; [DisplayDevices]::EnumDisplayDevices($null, [uint32]$i, [ref]$disp, 0); $i++) {
    if ($disp.DeviceName -eq $dn) { $foundDisplay = $disp; break }
    $disp.cb = [System.Runtime.InteropServices.Marshal]::SizeOf($disp)
  }

  $mon = [DisplayDevices]::GetDevice($dn, 0)

  # Extraire modelCode depuis DeviceID (souvent: MONITOR\\XXXXXXX\\...)
  $modelCode = $null
  if ($mon.DeviceID -match "MONITOR\\\\([^\\\\]+)\\\\") { $modelCode = $Matches[1] }

  $picked = $null
  if ($modelCode -and $wmiByModel.ContainsKey($modelCode) -and $wmiByModel[$modelCode].Count -gt 0) {
    # best-effort: pop la 1ère entrée de ce modèle
    $picked = $wmiByModel[$modelCode][0]
    $wmiByModel[$modelCode].RemoveAt(0)
  }

  $out += [PSCustomObject]@{
    DeviceName = $dn
    Primary = $s.Primary
    Bounds = @{
      x = $s.Bounds.X
      y = $s.Bounds.Y
      width = $s.Bounds.Width
      height = $s.Bounds.Height
    }
    MonitorDeviceID = $mon.DeviceID
    ModelCode = $modelCode
    InstanceName = $picked.InstanceName
    FriendlyName = $picked.FriendlyName
    Serial = $picked.Serial
  }
}

$out | ConvertTo-Json -Compress
`.trim();

    execFile(
      'powershell.exe',
      ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', ps],
      { windowsHide: true, maxBuffer: 5 * 1024 * 1024 },
      (err, stdout) => {
        if (err) return resolve([]);
        try {
          const parsed = JSON.parse((stdout || '').trim() || '[]');
          resolve(Array.isArray(parsed) ? parsed : [parsed]);
        } catch {
          resolve([]);
        }
      }
    );
  });
}

function findWinScreenForElectronDisplay(display, winScreens) {
  const b = display.bounds;
  return winScreens.find(ws => sameBounds(ws.Bounds, b)) || null;
}

// -------- RESOLUTION ECRAN AU DEMARRAGE --------

async function resolveTargetDisplayFromConfigAsync() {
  const cfg = loadConfig();
  const displays = screen.getAllDisplays();

  // 1) Windows robuste: instanceName (si dispo)
  if (process.platform === 'win32' && cfg.winMonitor) {
    const winScreens = await getWinScreensDetailed();

    // match par instanceName si présent
    let ws = null;
    if (cfg.winMonitor.instanceName) {
      ws = winScreens.find(x => x.InstanceName && x.InstanceName === cfg.winMonitor.instanceName) || null;
    }

    // sinon match par modelCode+serial
    if (!ws && cfg.winMonitor.modelCode) {
      ws = winScreens.find(x => x.ModelCode === cfg.winMonitor.modelCode && (x.Serial || '') === (cfg.winMonitor.serial || '')) || null;
    }

    // sinon match par friendlyName (moins robuste, fallback)
    if (!ws && cfg.winMonitor.friendlyName) {
      ws = winScreens.find(x => x.FriendlyName && x.FriendlyName === cfg.winMonitor.friendlyName) || null;
    }

    if (ws && ws.Bounds) {
      const byBounds = displays.find(d => sameBounds(d.bounds, ws.Bounds));
      if (byBounds) return byBounds;
    }
  }

  // 2) Electron id (souvent OK intra-session)
  if (cfg.displayId) {
    const byId = displays.find(d => d.id === cfg.displayId);
    if (byId) return byId;
  }

  // 3) key géométrique
  if (cfg.displayKey) {
    const byKey = displays.find(d => makeDisplayKey(d) === cfg.displayKey);
    if (byKey) return byKey;
  }

  // 4) index
  if (Number.isInteger(cfg.displayIndex) && displays[cfg.displayIndex]) {
    return displays[cfg.displayIndex];
  }

  // 5) primary
  return screen.getPrimaryDisplay();
}

// -------- ALWAYS ON TOP LOOP --------

function startKeepOnTopLoop() {
  stopKeepOnTopLoop();
  keepOnTopInterval = setInterval(() => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.setAlwaysOnTop(true, 'screen-saver');
    }
  }, 2000);
}

function stopKeepOnTopLoop() {
  if (keepOnTopInterval) {
    clearInterval(keepOnTopInterval);
    keepOnTopInterval = null;
  }
}

// -------- WINDOW LIFECYCLE --------

async function createWindowAsync() {
  const cfg = loadConfig();
  const targetDisplay = await resolveTargetDisplayFromConfigAsync();

  // Persister les fallbacks Electron dès qu’on sait où on démarre
  const all = screen.getAllDisplays();
  const idx = all.findIndex(d => d.id === targetDisplay.id);

  saveConfig({
    displayId: targetDisplay.id,
    displayKey: makeDisplayKey(targetDisplay),
    displayIndex: idx
  });

  const { width, height, x, y } = targetDisplay.bounds;

  mainWindow = new BrowserWindow({
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
      nodeIntegration: true,
      contextIsolation: false,
      backgroundThrottling: false,
      webSecurity: false,
      allowRunningInsecureContent: true
    }
  });

  startKeepOnTopLoop();

  // Permissions Audio/Vidéo
  mainWindow.webContents.session.setPermissionRequestHandler((webContents, permission, callback) => {
    callback(true);
  });

  // Injection Script Audio + Gestion du Volume Dynamique
  const startVolume = cfg.volume !== undefined ? cfg.volume : 1.0;

  mainWindow.webContents.on('did-finish-load', () => {
    mainWindow.webContents.executeJavaScript(`
      window.currentOverlayVolume = ${startVolume};

      const observer = new MutationObserver((mutations) => {
        mutations.forEach((mutation) => {
          mutation.addedNodes.forEach((node) => {
            if (node && (node.tagName === 'AUDIO' || node.tagName === 'VIDEO')) {
              node.muted = false;
              node.volume = window.currentOverlayVolume;
              node.play().catch(e => console.error("Erreur lecture auto:", e));
            }
          });
        });
      });
      observer.observe(document.body, { childList: true, subtree: true });

      const AudioContext = window.AudioContext || window.webkitAudioContext;
      if (AudioContext) {
        const ctx = new AudioContext();
        if (ctx.state === 'suspended') ctx.resume();
      }
    `);
  });

  mainWindow.loadURL('https://livechat.singesupreme.fr/client?guildId=942211187079282688');
  mainWindow.setIgnoreMouseEvents(true, { forward: true });

  mainWindow.once('ready-to-show', () => mainWindow.show());

  mainWindow.on('closed', () => {
    mainWindow = null;
    stopKeepOnTopLoop();
    if (tray) updateTrayMenu();
  });
}

function destroyWindow() {
  stopKeepOnTopLoop();
  if (mainWindow && !mainWindow.isDestroyed()) mainWindow.destroy();
  mainWindow = null;
}

async function setEnabledAsync(enabled) {
  saveConfig({ enabled });
  if (enabled) {
    if (!mainWindow) await createWindowAsync();
  } else {
    destroyWindow();
  }
  if (tray) updateTrayMenu();
}

// -------- CONTROLES (VOLUME / DISPLAY) --------

function changeVolume(level) {
  saveConfig({ volume: level });

  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.executeJavaScript(`
      window.currentOverlayVolume = ${level};
      document.querySelectorAll('audio, video').forEach(el => el.volume = ${level});
    `);
  }
  if (tray) updateTrayMenu();
}

async function moveWindowToDisplayAsync(display) {
  // On récupère les infos Windows (si possible) pour persister “matériel”
  let winScreens = [];
  if (process.platform === 'win32') {
    winScreens = await getWinScreensDetailed();
  }
  const ws = findWinScreenForElectronDisplay(display, winScreens);

  const all = screen.getAllDisplays();
  const index = all.findIndex(d => d.id === display.id);

  saveConfig({
    displayId: display.id,
    displayKey: makeDisplayKey(display),
    displayIndex: index,
    winMonitor: ws ? {
      instanceName: ws.InstanceName || null,
      modelCode: ws.ModelCode || null,
      serial: ws.Serial || null,
      friendlyName: ws.FriendlyName || null
    } : (loadConfig().winMonitor ?? null)
  });

  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.setBounds({
      x: display.bounds.x,
      y: display.bounds.y,
      width: display.bounds.width,
      height: display.bounds.height
    });
  }
  if (tray) updateTrayMenu();
}

function isSelectedDisplay(display, index) {
  const cfg = loadConfig();

  // Si l’id matche, ok
  if (cfg.displayId && display.id === cfg.displayId) return true;

  // Sinon key géométrique
  if (cfg.displayKey && makeDisplayKey(display) === cfg.displayKey) return true;

  // Sinon fallback index
  if (Number.isInteger(cfg.displayIndex) && cfg.displayIndex === index) return true;

  return false;
}

// -------- TRAY --------

function createTray() {
  const iconPath = path.join(__dirname, 'icon.png');
  const icon = nativeImage.createFromPath(iconPath);

  tray = new Tray(icon);
  tray.setToolTip('Overlay Audio/Video');

  updateTrayMenu();

  screen.on('display-added', updateTrayMenu);
  screen.on('display-removed', updateTrayMenu);
  screen.on('display-metrics-changed', updateTrayMenu);
}

async function updateTrayMenuAsync() {
  if (!tray) return;

  const displays = screen.getAllDisplays();
  const cfg = loadConfig();
  const currentVol = cfg.volume !== undefined ? cfg.volume : 1.0;
  const enabled = cfg.enabled ?? true;

  // Pour afficher les noms d’écran Windows et fiabiliser la sélection
  const winScreens = (process.platform === 'win32') ? await getWinScreensDetailed() : [];

  const template = [
    {
      label: 'Overlay activé',
      type: 'checkbox',
      checked: enabled,
      click: ({ checked }) => setEnabledAsync(checked)
    },
    { type: 'separator' },

    { label: 'CONFIGURATION ÉCRAN', enabled: false },
    { type: 'separator' }
  ];

  displays.forEach((display, index) => {
    const ws = findWinScreenForElectronDisplay(display, winScreens);
    const friendly = ws?.FriendlyName ? ` : ${ws.FriendlyName}` : '';
    template.push({
      label: `Écran ${index + 1}${friendly} (${display.bounds.width}x${display.bounds.height})`,
      type: 'radio',
      checked: isSelectedDisplay(display, index),
      click: () => moveWindowToDisplayAsync(display)
    });
  });

  template.push({ type: 'separator' });

  template.push({
    label: 'Volume Audio',
    submenu: [
      { label: 'Muet (0%)', type: 'radio', checked: currentVol === 0, click: () => changeVolume(0) },
      { label: '25%', type: 'radio', checked: currentVol === 0.25, click: () => changeVolume(0.25) },
      { label: '50%', type: 'radio', checked: currentVol === 0.5, click: () => changeVolume(0.5) },
      { label: '75%', type: 'radio', checked: currentVol === 0.75, click: () => changeVolume(0.75) },
      { label: '100%', type: 'radio', checked: currentVol === 1, click: () => changeVolume(1) }
    ]
  });

  template.push({ type: 'separator' });

  template.push({
    label: 'Recharger (Shift+Echap)',
    enabled: enabled && !!mainWindow && !mainWindow.isDestroyed(),
    click: () => mainWindow && mainWindow.reload()
  });

  template.push({
    label: 'Quitter',
    click: () => {
      destroyWindow();
      app.quit();
    }
  });

  tray.setContextMenu(Menu.buildFromTemplate(template));
}

function updateTrayMenu() {
  updateTrayMenuAsync().catch(console.error);
}

// -------- SHORTCUTS --------

function registerShortcuts() {
  globalShortcut.register('Shift+Escape', () => {
    const cfg = loadConfig();
    if (cfg.enabled && mainWindow && !mainWindow.isDestroyed()) mainWindow.reload();
  });
}

// -------- INIT --------

app.whenReady().then(async () => {
  const cfg = loadConfig();

  createTray();
  registerShortcuts();

  if (cfg.enabled) await createWindowAsync();

  app.on('activate', async () => {
    const c = loadConfig();
    if (c.enabled && !mainWindow) await createWindowAsync();
  });
});

// Garder l'app vivante dans le tray même sans fenêtre
app.on('window-all-closed', () => {
  // Intentionnel
});

app.on('will-quit', () => {
  globalShortcut.unregisterAll();
  stopKeepOnTopLoop();
});
