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

use windows::core::{w, Result as WindowsResult, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HWND};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY,
};
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
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
