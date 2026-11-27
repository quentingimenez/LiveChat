//! Bloc Win32 isolé du shell overlay.
//!
//! Exigences du contrat Fenêtre (docs/rust/overlay-bridge.md §5.3) :
//! l'overlay doit rester au-dessus des autres fenêtres sans jamais recevoir le
//! focus, rester hors de la barre des tâches, être cliquable-au-travers et
//! conserver ses styles lorsque WebView2 ou l'utilisateur les modifient.
//!
//! Tauri/tao pose déjà `WS_EX_LAYERED | WS_EX_TRANSPARENT` via
//! `set_ignore_cursor_events(true)`, `WS_EX_NOACTIVATE` via `set_focusable(false)`
//! et `WS_EX_TOOLWINDOW` via `set_skip_taskbar(true)`. Ce module réapplique les
//! quatre styles en ceinture et bretelles (le POC Phase 0 a montré qu'ils
//! pouvaient manquer selon le timing du `show()`).
//!
//! Exception : pendant le placement de la zone d'affichage (`placement`),
//! [`clear_click_through`] rend l'overlay cliquable et activable ; la garde
//! topmost ne réimpose alors pas ces styles.
//!
//! Ce fichier est le SEUL du crate autorisé à utiliser `unsafe` : il est
//! documenté comme tel dans `docs/rust/lc-overlay-shell.md`.
#![allow(unsafe_code)]

use std::sync::OnceLock;

use windows::core::{w, Result as WindowsResult, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Gdi::{EnumDisplayDevicesW, DISPLAY_DEVICEW};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};
use windows::Win32::System::SystemInformation::{GetSystemFirmwareTable, RSMB};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, HWND_TOPMOST, OBJID_WINDOW,
    SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE,
    WINEVENT_OUTOFCONTEXT, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT,
};

/// Styles étendus requis par l'overlay transparent cliquable-au-travers.
pub const OVERLAY_EX_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(
    WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0,
);

/// Styles étendus à vérifier dans les logs après application.
pub fn describe_ex_style(ex_style: isize) -> String {
    let raw = ex_style as u32;
    let mut flags = Vec::new();
    if raw & WS_EX_LAYERED.0 != 0 {
        flags.push("WS_EX_LAYERED");
    }
    if raw & WS_EX_TRANSPARENT.0 != 0 {
        flags.push("WS_EX_TRANSPARENT");
    }
    if raw & WS_EX_NOACTIVATE.0 != 0 {
        flags.push("WS_EX_NOACTIVATE");
    }
    if raw & WS_EX_TOOLWINDOW.0 != 0 {
        flags.push("WS_EX_TOOLWINDOW");
    }
    if flags.is_empty() {
        format!("0x{raw:08X}")
    } else {
        format!("0x{raw:08X} [{}]", flags.join("|"))
    }
}

/// Retourne les styles étendus courants de la fenêtre.
pub fn ex_style(hwnd: HWND) -> isize {
    // SAFETY: `hwnd` provient d'une fenêtre Tauri vivante ; GetWindowLongPtrW ne
    // fait que lire l'état de la fenêtre.
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) }
}

/// Ajoute les styles overlay manquants (idempotent) et retourne les styles finaux.
pub fn ensure_overlay_ex_style(hwnd: HWND) -> isize {
    let current = ex_style(hwnd);
    let desired = (current as u32) | OVERLAY_EX_STYLE.0;
    if desired != current as u32 {
        // SAFETY: même HWND vivant ; l'appel ne modifie que les styles étendus et
        // le retour précédent est ignoré volontairement (échec = style inchangé).
        let _previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired as isize) };
        desired as isize
    } else {
        current
    }
}

/// Styles removed while the display zone is placed: the overlay must then
/// take clicks and keyboard focus.
pub const CLICK_THROUGH_EX_STYLE: WINDOW_EX_STYLE =
    WINDOW_EX_STYLE(WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0);

/// Ex style of the overlay during the zone placement (pure, testable).
pub fn placement_ex_style(current: u32) -> u32 {
    current & !CLICK_THROUGH_EX_STYLE.0
}

/// Makes the overlay clickable and activable for the zone placement; the
/// next [`ensure_overlay_ex_style`] makes it click-through again.
pub fn clear_click_through(hwnd: HWND) {
    let current = ex_style(hwnd) as u32;
    let desired = placement_ex_style(current);
    if desired != current {
        // SAFETY: same live HWND; only the extended styles change and the
        // previous value is deliberately ignored (failure = style unchanged).
        let _previous = unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired as isize) };
    }
}

/// Réaffirme `HWND_TOPMOST` sans activer ni déplacer la fenêtre.
pub fn reassert_topmost(hwnd: HWND) -> WindowsResult<()> {
    // SAFETY: `hwnd` provient d'une fenêtre Tauri vivante ; les flags interdisent
    // tout déplacement/redimensionnement/activation.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
}

/// `EDD_GET_DEVICE_INTERFACE_NAME` (GDI) : `EnumDisplayDevicesW` rend alors le
/// chemin d'interface du moniteur, qui nomme son instance de registre — la
/// seule clé sous laquelle Windows range l'EDID.
const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 0x0000_0001;

/// Classes of shell windows that must never get a window placed behind them:
/// the desktop sits at the bottom of the Z order (a window behind it would
/// vanish under the icons) and the taskbars are topmost.
const SHELL_WINDOW_CLASSES: [&str; 4] = [
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

/// Decides whether a window that just lost the focus may be put right behind
/// the new foreground window.
///
/// Only an ordinary window qualifies: not the window itself, not a topmost
/// window (inserting after it would keep ours topmost) and not a shell window.
pub fn may_settle_behind(foreground_class: &str, foreground_topmost: bool, is_self: bool) -> bool {
    !is_self && !foreground_topmost && !SHELL_WINDOW_CLASSES.contains(&foreground_class)
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0_u16; 256];
    // SAFETY: `hwnd` comes from GetForegroundWindow and the buffer outlives the
    // call; GetClassNameW writes at most `buffer.len()` UTF-16 units.
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    let length = usize::try_from(length).unwrap_or(0).min(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

/// Puts `hwnd` right behind the current foreground window, without activating
/// it.
///
/// Removing the topmost state with `HWND_NOTOPMOST` (what Tauri does) moves a
/// window above *every* ordinary window, including the one the user just
/// clicked: the board then stayed in front of it. Inserting the window after
/// the foreground one gives the normal Windows behaviour, and also clears its
/// topmost state. Returns `false` when the foreground window does not qualify
/// (see [`may_settle_behind`]).
pub fn place_behind_foreground(hwnd: HWND) -> bool {
    // SAFETY: GetForegroundWindow only reads the desktop's foreground window.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_invalid() {
        return false;
    }
    let foreground_topmost = (ex_style(foreground) as u32) & WS_EX_TOPMOST.0 != 0;
    if !may_settle_behind(
        &class_name(foreground),
        foreground_topmost,
        foreground == hwnd,
    ) {
        return false;
    }
    // SAFETY: `hwnd` comes from a live Tauri window; the flags forbid any
    // move, resize or activation, and the async flag never blocks the caller
    // on the window's thread.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(foreground),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
    .is_ok()
}

/// Shows a window without activating it (`SW_SHOWNOACTIVATE`).
///
/// Once a window exists, tao shows it — and re-shows it after any style
/// change — with `SW_SHOW`, which activates it even with `WS_EX_NOACTIVATE`:
/// `focused(false)` only applies to the show done at creation. tao's own
/// visibility flag is left unset, so no tao setter may run on the window
/// afterwards (it would hide it again); only destroy it.
pub fn show_without_activation(hwnd: HWND) {
    // SAFETY: `hwnd` comes from a live Tauri window. The return value only
    // says whether the window was visible before, so it is ignored.
    let _was_visible = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
}

/// Whether `hwnd` currently has the topmost extended style.
pub fn is_topmost(hwnd: HWND) -> bool {
    (ex_style(hwnd) as u32) & WS_EX_TOPMOST.0 != 0
}

/// The desktop's foreground window, if any.
pub fn foreground_window() -> Option<HWND> {
    // SAFETY: GetForegroundWindow only reads the desktop's foreground window.
    let foreground = unsafe { GetForegroundWindow() };
    (!foreground.is_invalid()).then_some(foreground)
}

type ForegroundCallback = Box<dyn Fn(HWND) + Send + Sync>;

static FOREGROUND_CALLBACK: OnceLock<ForegroundCallback> = OnceLock::new();

/// Calls `callback` with the new foreground window every time the desktop's
/// foreground window changes, in any process.
///
/// Must run on a thread that pumps messages (the main thread): Windows queues
/// out-of-context WinEvents to the thread that installed the hook, so the
/// callback always runs there. WebView2 does not reliably raise `LostFocus`
/// when another application is activated, so Tauri's `Focused(false)` event
/// cannot be relied on for this. Only one callback can be installed; returns
/// `false` if one already was or if the hook could not be set.
pub fn watch_foreground(callback: impl Fn(HWND) + Send + Sync + 'static) -> bool {
    if FOREGROUND_CALLBACK.set(Box::new(callback)).is_err() {
        return false;
    }
    // SAFETY: `foreground_event` is a plain `extern "system"` function with the
    // WINEVENTPROC signature; out-of-context hooks need no module handle. The
    // hook lives as long as the process, so it is never unhooked.
    let hook: HWINEVENTHOOK = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    !hook.is_invalid()
}

unsafe extern "system" fn foreground_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _thread_id: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || object_id != OBJID_WINDOW.0 || hwnd.is_invalid() {
        return;
    }
    if let Some(callback) = FOREGROUND_CALLBACK.get() {
        callback(hwnd);
    }
}

/// Whether the Windows taskbar uses the light theme (`SystemUsesLightTheme`).
///
/// The tray icon follows it: its framing corners must contrast with the
/// taskbar. Falls back to `false` (dark taskbar, the Windows 11 default) when
/// the value cannot be read.
pub fn taskbar_uses_light_theme() -> bool {
    let mut data: u32 = 0;
    let mut size = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4);
    // SAFETY: `data` and `size` outlive the call, `size` is the byte length of
    // `data`, and RRF_RT_REG_DWORD makes the API write at most one DWORD.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut data).cast()),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && data != 0
}

/// UUID SMBIOS de la carte mère (`GetSystemFirmwareTable('RSMB', 0, …)`).
///
/// L'UUID est un identifiant matériel : il survit à une réinstallation de
/// Windows, mais pas à un changement de carte mère. C'est une empreinte, pas
/// une identité : l'appelant la hache par guilde et ne doit jamais la
/// journaliser. `None` quand la table est illisible ou ne porte que des valeurs
/// d'usine.
pub fn smbios_board_uuid() -> Option<String> {
    // SAFETY: a null buffer with size 0 only asks for the required size.
    let size = unsafe { GetSystemFirmwareTable(RSMB, 0, None) };
    // A real SMBIOS table is far smaller; anything huge is a broken driver.
    if size == 0 || size > 1_048_576 {
        return None;
    }
    let mut buffer = vec![0_u8; usize::try_from(size).ok()?];
    // SAFETY: `buffer` is `size` bytes and the API writes at most `size`.
    let written = unsafe { GetSystemFirmwareTable(RSMB, 0, Some(&mut buffer)) };
    if written == 0 || written > size {
        return None;
    }
    buffer.truncate(usize::try_from(written).ok()?);
    smbios_board_uuid_from_table(&buffer)
}

/// Extrait l'UUID de la structure SMBIOS type 1 (`System Information`).
fn smbios_board_uuid_from_table(table: &[u8]) -> Option<String> {
    let mut offset = 8_usize; // en-tête RawSMBIOSData
    while offset + 4 <= table.len() {
        let structure_type = table[offset];
        let length = usize::from(table[offset + 1]);
        if length < 4 || offset + length > table.len() {
            return None;
        }
        if structure_type == 1 && length >= 0x19 {
            let uuid: [u8; 16] = table.get(offset + 8..offset + 24)?.try_into().ok()?;
            return uuid_hex(&uuid);
        }
        // Zone formatée, puis jeu de chaînes terminé par une chaîne vide.
        let mut next = offset + length;
        while next + 1 < table.len() {
            if table[next] == 0 && table[next + 1] == 0 {
                next += 2;
                break;
            }
            next += 1;
        }
        offset = next;
    }
    None
}

/// 32 caractères hexadécimaux, sauf les valeurs d'usine tout-0 et tout-FF.
fn uuid_hex(uuid: &[u8; 16]) -> Option<String> {
    if uuid.iter().all(|byte| *byte == 0) || uuid.iter().all(|byte| *byte == 0xff) {
        return None;
    }
    let hex: String = uuid.iter().map(|byte| format!("{byte:02x}")).collect();
    (hex.len() == 32).then_some(hex)
}

/// Numéro de série du disque système (`\\.\PhysicalDrive0`).
///
/// Best-effort : l'accès direct au disque peut être refusé (droits, disque
/// virtuel), auquel cas la série est simplement absente de l'empreinte. Le
/// numéro de série n'est jamais journalisé.
pub fn system_disk_serial() -> Option<String> {
    // SAFETY: the path is a static wide string and no security attributes are
    // passed; the returned handle is closed below.
    let handle = unsafe {
        CreateFileW(
            w!(r"\\.\PhysicalDrive0"),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .ok()?;
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buffer = vec![0_u8; 1024];
    let mut returned = 0_u32;
    // SAFETY: `query` and `buffer` outlive the call and their sizes are exact;
    // `handle` is valid until CloseHandle.
    let result = unsafe {
        DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(std::ptr::from_ref(&query).cast()),
            u32::try_from(std::mem::size_of::<STORAGE_PROPERTY_QUERY>()).ok()?,
            Some(buffer.as_mut_ptr().cast()),
            u32::try_from(buffer.len()).ok()?,
            Some(&mut returned),
            None,
        )
    };
    // SAFETY: `handle` comes from CreateFileW and is closed exactly once.
    drop(unsafe { CloseHandle(handle) });
    result.ok()?;
    let returned = usize::try_from(returned).ok()?.min(buffer.len());
    if returned < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return None;
    }
    // SAFETY: the buffer holds at least the fixed descriptor; `read_unaligned`
    // tolerates the byte alignment of the Vec allocation.
    let descriptor =
        unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>()) };
    let offset = usize::try_from(descriptor.SerialNumberOffset).ok()?;
    if offset == 0 || offset >= returned {
        return None;
    }
    let bytes = &buffer[offset..returned];
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let serial = String::from_utf8_lossy(&bytes[..end]);
    let serial = serial.trim();
    (!serial.is_empty()).then(|| serial.to_owned())
}

/// Modèle du processeur (`ProcessorNameString`), stable pour une machine.
pub fn processor_name() -> Option<String> {
    read_machine_string(
        w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"),
        w!("ProcessorNameString"),
    )
}

/// Lit une valeur `REG_SZ` de `HKLM` (jamais journalisée).
fn read_machine_string(subkey: PCWSTR, value: PCWSTR) -> Option<String> {
    let mut buffer = [0_u16; 128];
    let mut size = u32::try_from(buffer.len() * 2).ok()?;
    // SAFETY: `buffer` and `size` outlive the call, `size` is the byte length
    // of `buffer`, and `RRF_RT_REG_SZ` makes the API write a NUL-terminated
    // UTF-16 string no longer than that.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey,
            value,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let units = usize::try_from(size).ok()? / std::mem::size_of::<u16>();
    let units = units.min(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..units]);
    let trimmed = value.trim_end_matches('\0').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Identité matérielle du moniteur : constructeur, produit et numéro de série
/// de l'EDID (`DEL:41B2:065KXLX123456`).
///
/// C'est la seule identité que Windows ne réattribue pas : ni `\\.\DISPLAYn`
/// (renuméroté au gré des branchements) ni la géométrie (échangée dès que les
/// écrans bougent). Un moniteur sans numéro de série lisible est identifié par
/// son instance de registre, stable tant que le pilote ne le réinstalle pas.
pub fn display_hardware_id(device_name: &str) -> Option<String> {
    // Le chemin d'interface (`\\?\DISPLAY#SAM707A#7&…&UID772#{…}`) nomme
    // l'instance exacte du registre ; `DeviceID` simple (`MONITOR\SAM707A\{…}\0008`)
    // ne porte qu'un index de classe sous lequel aucun EDID n'est rangé.
    let device_id = enum_display_device(device_name, 0, EDD_GET_DEVICE_INTERFACE_NAME)
        .and_then(|device| device_string(&device.DeviceID))
        .or_else(|| {
            enum_display_device(device_name, 0, 0)
                .and_then(|device| device_string(&device.DeviceID))
        })?;
    let (hardware_id, instance) = monitor_device_path(&device_id)?;
    let edid = registry_monitor_edid(&hardware_id, &instance)?;
    edid_hardware_id(&edid, &instance)
}

/// Nom court du moniteur tel que Windows l'affiche (ex. « LC27G5xT »).
///
/// `DeviceString` ne porte le modèle que si le constructeur fournit un pilote
/// nommé ; sinon il vaut « Generic PnP Monitor ». Le nom affiché par Windows
/// vient de l'EDID : on relit le descripteur 0xFC du moniteur rattaché à
/// l'adaptateur `\\.\DISPLAYn`, avec `DeviceString` en dernier recours.
/// Purement cosmétique : jamais persisté, ni clé ni identité d'écran.
pub fn display_short_name(device_name: &str) -> Option<String> {
    monitor_hardware_id(device_name)
        .and_then(|hardware_id| registry_monitor_name(&hardware_id))
        .or_else(|| {
            enum_display_device(device_name, 0, 0)
                .and_then(|device| device_string(&device.DeviceString))
        })
}

/// Premier moniteur rattaché à l'adaptateur, tel que le rapporte GDI.
fn enum_display_device(device_name: &str, index: u32, flags: u32) -> Option<DISPLAY_DEVICEW> {
    let device_name = device_name.trim();
    if device_name.is_empty() {
        return None;
    }
    let wide: Vec<u16> = device_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut device = DISPLAY_DEVICEW {
        cb: u32::try_from(std::mem::size_of::<DISPLAY_DEVICEW>()).ok()?,
        ..Default::default()
    };
    // SAFETY: `wide` is NUL-terminated and outlives the call; `device` is a
    // valid `DISPLAY_DEVICEW` whose `cb` describes its size.
    let found =
        unsafe { EnumDisplayDevicesW(PCWSTR::from_raw(wide.as_ptr()), index, &mut device, flags) };
    found.as_bool().then_some(device)
}

/// Identifiant matériel du moniteur : `SAM707A` dans `MONITOR\SAM707A\{…}\0008`.
fn monitor_hardware_id(device_name: &str) -> Option<String> {
    enum_display_device(device_name, 0, 0)
        .and_then(|device| device_string(&device.DeviceID))
        .and_then(|device_id| hardware_id_from_device_id(&device_id))
}

fn hardware_id_from_device_id(device_id: &str) -> Option<String> {
    let mut parts = device_id.split('\\');
    let _bus = parts.next()?;
    let hardware_id = parts.next()?.trim();
    (!hardware_id.is_empty()).then(|| hardware_id.to_owned())
}

/// Identifiant matériel et instance du moniteur dans le registre.
///
/// Deux formes de `DeviceID` désignent le même écran :
/// `MONITOR\SAM707A\{4d36e96e-…}\0008` quand GDI nomme l'adaptateur, et
/// `\\?\DISPLAY#SAM707A#7&2dc4a82b&1&UID772#{e6f07b5f-…}` avec
/// `EDD_GET_DEVICE_INTERFACE_NAME`, où l'instance est enfin le nom de clé sous
/// `Enum\DISPLAY\<hardware_id>` — le seul endroit où Windows range l'EDID.
fn monitor_device_path(device_id: &str) -> Option<(String, String)> {
    let device_id = device_id.trim();
    let (hardware_id, instance) = match device_id.strip_prefix(r"\\?\") {
        // `\\?\DISPLAY#<hardware_id>#<instance>#{interface}`.
        Some(interface) => {
            let parts: Vec<&str> = interface.split('#').collect();
            (*parts.get(1)?, *parts.get(2)?)
        }
        // `MONITOR\<hardware_id>\{class}\<instance>`.
        None => {
            let parts: Vec<&str> = device_id.split('\\').collect();
            (*parts.get(1)?, *parts.get(3)?)
        }
    };
    let (hardware_id, instance) = (hardware_id.trim(), instance.trim());
    // Un identifiant d'interface à la place de l'instance (`…\DISPLAY#SAM707A#{…}`)
    // n'a pas de nom de clé à ouvrir.
    if hardware_id.is_empty() || instance.is_empty() || instance.starts_with('{') {
        return None;
    }
    Some((hardware_id.to_owned(), instance.to_owned()))
}

/// Nom du moniteur lu dans l'EDID exposé par le registre des périphériques.
fn registry_monitor_name(hardware_id: &str) -> Option<String> {
    with_monitor_registry(hardware_id, scan_monitor_instances)
}

/// Bloc EDID de l'instance de moniteur qui porte exactement `device_id`.
fn registry_monitor_edid(hardware_id: &str, instance: &str) -> Option<Vec<u8>> {
    with_monitor_registry(hardware_id, |key| read_monitor_edid(key, instance))
}

/// Ouvre `HKLM\SYSTEM\CurrentControlSet\Enum\DISPLAY\<hardware_id>` le temps
/// d'une lecture.
fn with_monitor_registry<T>(hardware_id: &str, read: impl FnOnce(HKEY) -> Option<T>) -> Option<T> {
    let subkey: Vec<u16> = format!("SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\{hardware_id}")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut key = HKEY::default();
    // SAFETY: the subkey string is NUL-terminated and outlives the call.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR::from_raw(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let value = read(key);
    // SAFETY: `key` was opened above and is closed exactly once.
    let _ = unsafe { RegCloseKey(key) };
    value
}

/// Parcourt les instances du moniteur et retient la première qui porte un EDID.
fn scan_monitor_instances(key: HKEY) -> Option<String> {
    let mut index = 0_u32;
    loop {
        let mut buffer = [0_u16; 128];
        let mut length = u32::try_from(buffer.len()).unwrap_or_default();
        // SAFETY: `buffer` holds `length` UTF-16 units and the API writes at
        // most that many; `length` receives the count without the NUL.
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                Some(PWSTR(buffer.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        index += 1;
        let units = usize::try_from(length)
            .unwrap_or_default()
            .min(buffer.len());
        let instance = String::from_utf16_lossy(&buffer[..units]);
        if let Some(edid) = read_monitor_edid(key, &instance) {
            if let Some(name) = edid_monitor_name(&edid) {
                return Some(name);
            }
        }
    }
}

/// Bloc `EDID` de `…\<instance>\Device Parameters`.
fn read_monitor_edid(key: HKEY, instance: &str) -> Option<Vec<u8>> {
    let subkey: Vec<u16> = format!("{instance}\\Device Parameters")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let subkey = PCWSTR::from_raw(subkey.as_ptr());
    let mut size = 0_u32;
    // SAFETY: a null buffer with a size pointer only asks for the byte length.
    let status = unsafe {
        RegGetValueW(
            key,
            subkey,
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size == 0 || size > 4096 {
        return None;
    }
    let mut data = vec![0_u8; usize::try_from(size).ok()?];
    // SAFETY: `data` is `size` bytes and the API writes at most that.
    let status = unsafe {
        RegGetValueW(
            key,
            subkey,
            w!("EDID"),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    data.truncate(usize::try_from(size).ok()?);
    Some(data)
}

/// Nom du moniteur dans les descripteurs EDID (tag 0xFC, 13 octets ASCII).
fn edid_monitor_name(edid: &[u8]) -> Option<String> {
    edid_descriptor_text(edid, 0xfc)
}

/// Texte d'un descripteur EDID à 18 octets portant `tag`.
fn edid_descriptor_text(edid: &[u8], tag: u8) -> Option<String> {
    let descriptors = edid.get(54..)?.as_chunks::<18>().0.iter().take(4);
    for descriptor in descriptors {
        if descriptor[..3] != [0, 0, 0] || descriptor[3] != tag {
            continue;
        }
        let text: String = descriptor[5..18]
            .iter()
            .take_while(|byte| !matches!(**byte, 0x0a | 0x00 | 0xff))
            .map(|byte| char::from(*byte))
            .collect();
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_owned());
        }
    }
    None
}

/// Identité stable tirée de l'EDID : `CONSTRUCTEUR:PRODUIT:SÉRIE`.
///
/// Le constructeur est le code PnP trois lettres des octets 8-9, le produit son
/// identifiant des octets 10-11, la série le descripteur 0xFF (texte) sinon
/// l'identifiant numérique des octets 12-15. `instance` ne sert que pour un
/// moniteur muet sur les deux (EDID minimal de certains KVM).
fn edid_hardware_id(edid: &[u8], instance: &str) -> Option<String> {
    // Un bloc sans en-tête EDID valide ne désigne aucun moniteur.
    (edid.get(..8)? == [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]).then_some(())?;
    let vendor = edid_vendor(edid.get(8..10)?)?;
    let product = u16::from_le_bytes([*edid.get(10)?, *edid.get(11)?]);
    let serial = u32::from_le_bytes([
        *edid.get(12)?,
        *edid.get(13)?,
        *edid.get(14)?,
        *edid.get(15)?,
    ]);
    let serial = edid_descriptor_text(edid, 0xff).unwrap_or_else(|| {
        if serial == 0 || serial == u32::MAX {
            instance.to_owned()
        } else {
            format!("{serial:08X}")
        }
    });
    Some(format!("{vendor}:{product:04X}:{serial}"))
}

/// Code PnP trois lettres du constructeur EDID (octets 8-9, 5 bits par lettre,
/// 1 = « A »).
fn edid_vendor(bytes: &[u8]) -> Option<String> {
    let value = (u16::from(*bytes.first()?) << 8) | u16::from(*bytes.get(1)?);
    let letter = |bits: u16| -> Option<char> {
        match bits & 0x1f {
            0 => None,
            code => char::from_u32(u32::from('A') + u32::from(code) - 1),
        }
    };
    let mut vendor = String::with_capacity(3);
    vendor.push(letter(value >> 10)?);
    vendor.push(letter(value >> 5)?);
    vendor.push(letter(value)?);
    Some(vendor)
}

/// Chaîne UTF-16 terminée par NUL → texte trimé, `None` si vide.
fn device_string(units: &[u16]) -> Option<String> {
    let text = String::from_utf16_lossy(units);
    let text = text.split('\0').next().unwrap_or_default().trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_only_behind_an_ordinary_foreground_window() {
        assert!(may_settle_behind("Notepad", false, false));
        assert!(!may_settle_behind("Notepad", false, true), "itself");
        assert!(!may_settle_behind("Notepad", true, false), "topmost");
        assert!(!may_settle_behind("Progman", false, false), "desktop");
        assert!(!may_settle_behind("WorkerW", false, false), "desktop");
        assert!(!may_settle_behind("Shell_TrayWnd", false, false), "taskbar");
    }

    #[test]
    fn describe_reports_each_overlay_flag() {
        let described = describe_ex_style(OVERLAY_EX_STYLE.0 as isize);
        assert!(described.contains("WS_EX_LAYERED"));
        assert!(described.contains("WS_EX_TRANSPARENT"));
        assert!(described.contains("WS_EX_NOACTIVATE"));
        assert!(described.contains("WS_EX_TOOLWINDOW"));
    }

    #[test]
    fn the_placement_style_takes_clicks_and_keeps_the_other_flags() {
        let placing = placement_ex_style(OVERLAY_EX_STYLE.0 | WS_EX_TOPMOST.0);
        assert_eq!(placing & WS_EX_TRANSPARENT.0, 0);
        assert_eq!(placing & WS_EX_NOACTIVATE.0, 0);
        assert_ne!(placing & WS_EX_LAYERED.0, 0);
        assert_ne!(placing & WS_EX_TOOLWINDOW.0, 0);
        assert_ne!(placing & WS_EX_TOPMOST.0, 0);
    }

    #[test]
    fn describe_falls_back_to_raw_hex() {
        assert_eq!(describe_ex_style(0), "0x00000000");
    }

    #[test]
    fn smbios_parser_reads_the_board_uuid() {
        let uuid = [
            0x4c, 0x4c, 0x45, 0x44, 0x00, 0x31, 0x35, 0x10, 0x80, 0x43, 0xb2, 0xc0, 0x4f, 0x4d,
            0x4e, 0x31,
        ];
        assert_eq!(
            smbios_board_uuid_from_table(&smbios_table(uuid)).as_deref(),
            Some("4c4c4544003135108043b2c04f4d4e31")
        );
    }

    #[test]
    fn smbios_parser_skips_unusable_uuids_and_tables() {
        assert!(smbios_board_uuid_from_table(&smbios_table([0x00; 16])).is_none());
        assert!(smbios_board_uuid_from_table(&smbios_table([0xff; 16])).is_none());
        assert!(smbios_board_uuid_from_table(&[]).is_none());
        assert!(smbios_board_uuid_from_table(&[0_u8; 8]).is_none());
    }

    #[test]
    fn device_string_stops_at_the_first_nul_and_trims() {
        let units: Vec<u16> = "  G27q-20\u{0}garbage".encode_utf16().collect();
        assert_eq!(device_string(&units).as_deref(), Some("G27q-20"));
        assert_eq!(device_string(&[0_u16; 4]), None);
        assert_eq!(device_string(&[]), None);
    }

    #[test]
    fn hardware_id_is_the_monitor_id_of_the_device_path() {
        let device_id = "MONITOR\\SAM707A\\{4d36e96e-e325-11ce-bfc1-08002be10318}\\0008";
        assert_eq!(
            hardware_id_from_device_id(device_id).as_deref(),
            Some("SAM707A")
        );
        assert_eq!(hardware_id_from_device_id("MONITOR"), None);
        assert_eq!(hardware_id_from_device_id("MONITOR\\\\\\0008"), None);
    }

    #[test]
    fn edid_monitor_name_reads_the_fc_descriptor() {
        assert_eq!(
            edid_monitor_name(&edid_with_name("LC27G5xT\n   ")).as_deref(),
            Some("LC27G5xT")
        );
        // Un autre descripteur (0xFE) ne porte pas le nom.
        assert_eq!(
            edid_monitor_name(&edid_with_descriptor(0xfe, "week 12")),
            None
        );
        assert_eq!(edid_monitor_name(&[]), None);
        assert_eq!(edid_monitor_name(&[0_u8; 60]), None);
    }

    #[test]
    fn the_monitor_device_path_names_the_registry_instance() {
        let device_id = "MONITOR\\SAM707A\\{4d36e96e-e325-11ce-bfc1-08002be10318}\\0008";
        assert_eq!(
            monitor_device_path(device_id),
            Some(("SAM707A".to_string(), "0008".to_string()))
        );
        assert_eq!(monitor_device_path("MONITOR\\SAM707A"), None);
        assert_eq!(
            monitor_device_path("MONITOR\\SAM707A\\{4d36e96e-e325-11ce-bfc1-08002be10318}"),
            None
        );
    }

    #[test]
    fn the_interface_path_names_the_instance_holding_the_edid() {
        // Forme rendue avec `EDD_GET_DEVICE_INTERFACE_NAME` : l'instance y est
        // le nom de clé sous `Enum\DISPLAY\SAM707A`, seul endroit où l'EDID est
        // rangé (`MONITOR\SAM707A\{…}\0008` n'est qu'un index de classe).
        let interface = concat!(
            "\\\\?\\DISPLAY#SAM707A#7&2dc4a82b&1&UID772#",
            "{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"
        );
        assert_eq!(
            monitor_device_path(interface),
            Some(("SAM707A".to_string(), "7&2dc4a82b&1&UID772".to_string()))
        );
        // Une interface tronquée n'invente pas d'instance.
        assert_eq!(
            monitor_device_path("\\\\?\\DISPLAY#SAM707A#{e6f07b5f-ee97-4a90-b076-33f57bf4eaa7}"),
            None
        );
    }

    #[test]
    fn the_hardware_identity_comes_from_the_edid_header() {
        let edid = edid_with_identity(0x10ac, 0x41b2, 0x1234_5678);
        assert_eq!(
            edid_hardware_id(&edid, "0008").as_deref(),
            Some("DEL:41B2:12345678")
        );
        // Le numéro de série texte du descripteur 0xFF est plus précis que le
        // numérique : certains constructeurs y laissent 0 pour tous leurs
        // écrans d'un même modèle.
        let named = edid_with_descriptor(0xff, "Z9XK400001");
        assert_eq!(
            edid_hardware_id(&named, "0008").as_deref(),
            Some("DEL:41B2:Z9XK400001")
        );
    }

    #[test]
    fn a_monitor_without_a_serial_number_keeps_its_registry_instance() {
        let anonymous = edid_with_identity(0x10ac, 0x41b2, 0);
        assert_eq!(
            edid_hardware_id(&anonymous, "5&1a2b3c4d&0&1").as_deref(),
            Some("DEL:41B2:5&1a2b3c4d&0&1")
        );
        let all_ones = edid_with_identity(0x10ac, 0x41b2, u32::MAX);
        assert_eq!(
            edid_hardware_id(&all_ones, "0008").as_deref(),
            Some("DEL:41B2:0008")
        );
    }

    #[test]
    fn a_corrupt_edid_designates_no_monitor() {
        assert_eq!(edid_hardware_id(&[0_u8; 128], "0008"), None);
        assert_eq!(edid_hardware_id(&[], "0008"), None);
        // Constructeur nul : pas d'identité fiable.
        let blank_vendor = edid_with_identity(0, 0x41b2, 1);
        assert_eq!(edid_hardware_id(&blank_vendor, "0008"), None);
        // En-tête EDID faux.
        let mut bad_header = edid_with_identity(0x10ac, 0x41b2, 1);
        bad_header[7] = 0x01;
        assert_eq!(edid_hardware_id(&bad_header, "0008"), None);
    }

    /// EDID minimale valide : en-tête, constructeur, produit et série numériques.
    fn edid_with_identity(vendor: u16, product: u16, serial: u32) -> Vec<u8> {
        let mut edid = vec![0_u8; 128];
        edid[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        edid[8..10].copy_from_slice(&vendor.to_be_bytes());
        edid[10..12].copy_from_slice(&product.to_le_bytes());
        edid[12..16].copy_from_slice(&serial.to_le_bytes());
        edid
    }

    /// EDID minimale : un seul descripteur, tag 0xFC, texte de 13 octets.
    fn edid_with_name(text: &str) -> Vec<u8> {
        edid_with_descriptor(0xfc, text)
    }

    fn edid_with_descriptor(tag: u8, text: &str) -> Vec<u8> {
        let mut edid = edid_with_identity(0x10ac, 0x41b2, 0x1234_5678);
        edid[54..57].copy_from_slice(&[0, 0, 0]);
        edid[57] = tag;
        let bytes = text.as_bytes();
        let len = bytes.len().min(13);
        edid[59..59 + len].copy_from_slice(&bytes[..len]);
        edid
    }

    /// Table `RawSMBIOSData` minimale avec une seule structure type 1.
    fn smbios_table(uuid: [u8; 16]) -> Vec<u8> {
        let mut table = vec![0_u8; 8];
        // En-tête (type 1, longueur 0x19) puis les quatre indices de chaînes
        // (`Manufacturer`, `Product`, `Version`, `Serial`).
        table.extend_from_slice(&[1, 0x19, 0, 0, 0, 0, 0, 0]);
        table.extend_from_slice(&uuid);
        table.push(0); // complète la longueur 0x19
        table.extend_from_slice(&[0, 0]);
        let length = u32::try_from(table.len() - 8).unwrap_or_default();
        table[4..8].copy_from_slice(&length.to_le_bytes());
        table
    }
}
