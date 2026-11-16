//! Windows, macOS and Linux: capture with `xcap`, input with `enigo`, clipboard with `arboard`.

use crate::keys::{self, Action, Mods, Named, Physical};
use crate::{
    encode_rgba, Access, EncodedFrame, PermissionKind, Permissions, Platform, PlatformError,
};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use omnidesk_proto::{
    Capabilities, DeviceKind, InputEvent, MouseButton, Os, ScreenInfo, StreamOptions, ViewCap,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xcap::Monitor;

/// How long a list of monitors is reused before asking the OS again.
const MONITOR_CACHE: Duration = Duration::from_secs(5);

enum ClipRequest {
    Get(mpsc::Sender<Option<String>>),
    Set(String),
}

pub struct Desktop {
    name: String,
    kind: DeviceKind,
    input: Mutex<Option<mpsc::Sender<InputEvent>>>,
    clipboard: Mutex<Option<mpsc::Sender<ClipRequest>>>,
}

impl Desktop {
    pub fn new() -> Desktop {
        #[cfg(target_os = "windows")]
        win::dpi_aware();
        Desktop {
            name: device_name(),
            kind: if has_battery() {
                DeviceKind::Laptop
            } else {
                DeviceKind::Desktop
            },
            input: Mutex::new(None),
            clipboard: Mutex::new(None),
        }
    }

    fn clipboard_sender(&self) -> Option<mpsc::Sender<ClipRequest>> {
        let mut slot = self.clipboard.lock().ok()?;
        if slot.is_none() {
            let (tx, rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("omnidesk-clipboard".into())
                .spawn(move || clipboard_loop(rx))
                .ok()?;
            *slot = Some(tx);
        }
        slot.clone()
    }
}

impl Platform for Desktop {
    fn os(&self) -> Os {
        if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }

    fn kind(&self) -> DeviceKind {
        self.kind
    }

    fn device_name(&self) -> String {
        self.name.clone()
    }

    fn capabilities(&self) -> Capabilities {
        let has_screen = !self.screens().is_empty();
        Capabilities {
            view: if has_screen {
                ViewCap::Always
            } else {
                ViewCap::None
            },
            control: has_screen,
            grab: false,
            display_sink: false,
            display_source: false,
            clipboard: true,
        }
    }

    fn screens(&self) -> Vec<ScreenInfo> {
        let Ok(monitors) = Monitor::all() else {
            return Vec::new();
        };
        monitors
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                Some(ScreenInfo {
                    index: i as u32,
                    name: screen_name(m, i),
                    x: m.x().ok()?,
                    y: m.y().ok()?,
                    width: m.width().ok()?,
                    height: m.height().ok()?,
                    scale: m.scale_factor().unwrap_or(1.0),
                    primary: m.is_primary().unwrap_or(i == 0),
                })
            })
            .collect()
    }

    fn capture(&self, screen: u32, options: StreamOptions) -> Result<EncodedFrame, PlatformError> {
        // Without Screen Recording a Mac returns only the wallpaper: say why instead.
        #[cfg(target_os = "macos")]
        if !mac::screen_allowed() {
            return Err(PlatformError::Permission(PermissionKind::Screen));
        }
        thread_local! {
            static MONITORS: RefCell<Option<(Instant, Vec<Monitor>)>> = const { RefCell::new(None) };
        }
        MONITORS.with(|cell| {
            let mut cache = cell.borrow_mut();
            if cache
                .as_ref()
                .is_none_or(|(at, _)| at.elapsed() > MONITOR_CACHE)
            {
                *cache = Some((
                    Instant::now(),
                    Monitor::all().map_err(|e| PlatformError::Capture(e.to_string()))?,
                ));
            }
            let monitor = cache
                .as_ref()
                .and_then(|(_, list)| list.get(screen as usize))
                .ok_or(PlatformError::NoSuchScreen(screen))?;
            match monitor.capture_image() {
                Ok(image) => {
                    let (w, h) = (image.width(), image.height());
                    encode_rgba(image.into_raw(), w, h, options)
                }
                Err(e) => {
                    // The display may have been unplugged or its mode changed: look again next time.
                    *cache = None;
                    Err(PlatformError::Capture(e.to_string()))
                }
            }
        })
    }

    fn input(&self, event: InputEvent) -> Result<(), PlatformError> {
        #[cfg(target_os = "macos")]
        if !mac::control_allowed() {
            return Err(PlatformError::Permission(PermissionKind::Control));
        }
        let mut slot = self
            .input
            .lock()
            .map_err(|_| PlatformError::Input("input thread crashed".into()))?;
        if slot.is_none() {
            let (tx, rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("omnidesk-input".into())
                .spawn(move || input_loop(rx))
                .map_err(|e| PlatformError::Input(e.to_string()))?;
            *slot = Some(tx);
        }
        let sent = slot
            .as_ref()
            .map(|tx| tx.send(event).is_ok())
            .unwrap_or(false);
        if !sent {
            // The thread stopped (no permission, no display). Try again on the next event.
            *slot = None;
            return Err(PlatformError::Input(
                "keyboard and mouse control is not available on this computer".into(),
            ));
        }
        Ok(())
    }

    fn permissions(&self) -> Permissions {
        #[cfg(target_os = "macos")]
        {
            let state = |ok: bool| if ok { Access::Granted } else { Access::Missing };
            Permissions {
                screen: state(mac::screen_allowed()),
                control: state(mac::control_allowed()),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = Access::Granted;
            Permissions::NOT_NEEDED
        }
    }

    fn request_permission(&self, kind: PermissionKind) {
        #[cfg(target_os = "macos")]
        mac::request(kind);
        #[cfg(not(target_os = "macos"))]
        let _ = kind;
    }

    fn clipboard_get(&self) -> Option<String> {
        let (tx, rx) = mpsc::channel();
        self.clipboard_sender()?.send(ClipRequest::Get(tx)).ok()?;
        rx.recv_timeout(Duration::from_millis(800)).ok().flatten()
    }

    fn clipboard_set(&self, text: &str) {
        if let Some(tx) = self.clipboard_sender() {
            let _ = tx.send(ClipRequest::Set(text.to_string()));
        }
    }
}

fn clipboard_loop(rx: mpsc::Receiver<ClipRequest>) {
    let mut clipboard: Option<arboard::Clipboard> = None;
    for request in rx {
        if clipboard.is_none() {
            clipboard = arboard::Clipboard::new().ok();
        }
        let Some(cb) = clipboard.as_mut() else {
            if let ClipRequest::Get(reply) = request {
                let _ = reply.send(None);
            }
            continue;
        };
        match request {
            ClipRequest::Get(reply) => {
                let _ = reply.send(cb.get_text().ok());
            }
            ClipRequest::Set(text) => {
                if let Err(e) = cb.set_text(text) {
                    tracing::warn!("clipboard write failed: {e}");
                    clipboard = None;
                }
            }
        }
    }
}

fn monitor_rects() -> Vec<(i32, i32, u32, u32)> {
    Monitor::all()
        .map(|list| {
            list.iter()
                .filter_map(|m| Some((m.x().ok()?, m.y().ok()?, m.width().ok()?, m.height().ok()?)))
                .collect()
        })
        .unwrap_or_default()
}

fn to_key(named: Named) -> Key {
    match named {
        Named::Enter => Key::Return,
        Named::Tab => Key::Tab,
        Named::Backspace => Key::Backspace,
        Named::Escape => Key::Escape,
        Named::Delete => Key::Delete,
        Named::Home => Key::Home,
        Named::End => Key::End,
        Named::PageUp => Key::PageUp,
        Named::PageDown => Key::PageDown,
        Named::Left => Key::LeftArrow,
        Named::Right => Key::RightArrow,
        Named::Up => Key::UpArrow,
        Named::Down => Key::DownArrow,
        Named::Space => Key::Space,
        Named::CapsLock => Key::CapsLock,
        Named::ShiftLeft => Key::LShift,
        Named::ShiftRight => Key::RShift,
        Named::ControlLeft => Key::LControl,
        Named::ControlRight => Key::RControl,
        Named::Alt => Key::Alt,
        Named::Meta => Key::Meta,
        Named::VolumeUp => Key::VolumeUp,
        Named::VolumeDown => Key::VolumeDown,
        Named::VolumeMute => Key::VolumeMute,
        Named::MediaNext => Key::MediaNextTrack,
        Named::MediaPrev => Key::MediaPrevTrack,
        Named::MediaPlayPause => Key::MediaPlayPause,
        Named::F(n) => match n {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            6 => Key::F6,
            7 => Key::F7,
            8 => Key::F8,
            9 => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            12 => Key::F12,
            13 => Key::F13,
            14 => Key::F14,
            15 => Key::F15,
            16 => Key::F16,
            17 => Key::F17,
            18 => Key::F18,
            19 => Key::F19,
            _ => Key::F20,
        },
        Named::Numpad(n) => match n {
            0 => Key::Numpad0,
            1 => Key::Numpad1,
            2 => Key::Numpad2,
            3 => Key::Numpad3,
            4 => Key::Numpad4,
            5 => Key::Numpad5,
            6 => Key::Numpad6,
            7 => Key::Numpad7,
            8 => Key::Numpad8,
            _ => Key::Numpad9,
        },
    }
}

fn to_button(button: MouseButton) -> Button {
    match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
        MouseButton::Back => Button::Back,
        MouseButton::Forward => Button::Forward,
    }
}

/// Owns the OS input connection. One thread, because the connection is not thread-safe everywhere.
fn input_loop(rx: mpsc::Receiver<InputEvent>) {
    // No system prompt from the input library: the app shows its own guidance.
    let settings = Settings {
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    };
    let mut enigo = match Enigo::new(&settings) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("keyboard and mouse control is unavailable: {e}");
            return;
        }
    };
    // Key that was pressed for each physical key, so the release matches even if modifiers changed.
    let mut held: HashMap<String, Key> = HashMap::new();
    let mut monitors = monitor_rects();
    let mut monitors_at = Instant::now();
    let mut pending: Option<InputEvent> = None;

    loop {
        let mut event = match pending.take() {
            Some(e) => e,
            None => match rx.recv() {
                Ok(e) => e,
                Err(_) => break,
            },
        };
        // Pointer positions pile up faster than they can be played: keep only the newest.
        if matches!(event, InputEvent::PointerAbs { .. }) {
            while let Ok(next) = rx.try_recv() {
                if matches!(next, InputEvent::PointerAbs { .. }) {
                    event = next;
                } else {
                    pending = Some(next);
                    break;
                }
            }
        }
        let result = match event {
            InputEvent::PointerAbs { screen, x, y } => {
                if monitors_at.elapsed() > MONITOR_CACHE || monitors.is_empty() {
                    monitors = monitor_rects();
                    monitors_at = Instant::now();
                }
                match monitors.get(screen as usize).copied() {
                    Some((mx, my, mw, mh)) => {
                        let px =
                            mx + (x.clamp(0.0, 1.0) * mw.saturating_sub(1) as f32).round() as i32;
                        let py =
                            my + (y.clamp(0.0, 1.0) * mh.saturating_sub(1) as f32).round() as i32;
                        move_abs(&mut enigo, px, py)
                    }
                    None => Ok(()),
                }
            }
            InputEvent::PointerRel { dx, dy } => enigo.move_mouse(dx, dy, Coordinate::Rel),
            InputEvent::Button { button, down } => enigo.button(
                to_button(button),
                if down {
                    Direction::Press
                } else {
                    Direction::Release
                },
            ),
            InputEvent::Scroll { dx, dy } => {
                let mut r = Ok(());
                if dy != 0 {
                    r = enigo.scroll(dy, Axis::Vertical);
                }
                if dx != 0 {
                    r = r.and(enigo.scroll(dx, Axis::Horizontal));
                }
                r
            }
            InputEvent::Key { code, key, down } => {
                if down {
                    let mods = Mods::from_codes(held.keys().map(String::as_str));
                    match keys::decide(&code, &key, mods) {
                        Action::Named(n) => {
                            let k = to_key(n);
                            held.insert(code, k);
                            enigo.key(k, Direction::Press)
                        }
                        Action::Shortcut(p) => {
                            let k = physical_key(p);
                            held.insert(code, k);
                            enigo.key(k, Direction::Press)
                        }
                        Action::Text(c) => enigo.text(c.encode_utf8(&mut [0u8; 4])),
                        Action::Ignore => Ok(()),
                    }
                } else {
                    match held.remove(&code) {
                        Some(k) => enigo.key(k, Direction::Release),
                        None => Ok(()),
                    }
                }
            }
            InputEvent::Text { text } => enigo.text(&text),
        };
        if let Err(e) = result {
            tracing::debug!("input event failed: {e}");
        }
    }
    for (_, k) in held.drain() {
        let _ = enigo.key(k, Direction::Release);
    }
}

/// Moves the pointer to a point in desktop coordinates, on any monitor.
#[cfg(target_os = "windows")]
fn move_abs(_enigo: &mut Enigo, x: i32, y: i32) -> enigo::InputResult<()> {
    // The input library only addresses the main monitor; this covers the whole desktop.
    if win::move_pointer(x, y) {
        Ok(())
    } else {
        Err(enigo::InputError::Simulate(
            "the pointer could not be moved",
        ))
    }
}

#[cfg(not(target_os = "windows"))]
fn move_abs(enigo: &mut Enigo, x: i32, y: i32) -> enigo::InputResult<()> {
    enigo.move_mouse(x, y, Coordinate::Abs)
}

/// The key at a physical position, for shortcuts.
fn physical_key(p: Physical) -> Key {
    if cfg!(target_os = "windows") {
        Key::Other(p.windows_vk as u32)
    } else if cfg!(target_os = "macos") {
        Key::Other(p.mac_keycode as u32)
    } else {
        Key::Unicode(p.us)
    }
}

fn screen_name(m: &Monitor, index: usize) -> String {
    // On a Mac the display's marketing name needs AppKit, which may only be used from the main
    // thread. Built-in or numbered is enough to tell displays apart.
    if cfg!(target_os = "macos") {
        return if m.is_builtin().unwrap_or(false) {
            "Built-in Display".to_string()
        } else {
            format!("Display {}", index + 1)
        };
    }
    m.friendly_name()
        .or_else(|_| m.name())
        .ok()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Display {}", index + 1))
}

#[cfg(target_os = "windows")]
mod win {
    use windows_sys::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
        MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };

    /// Real pixels everywhere, so monitor positions, captures and pointer moves agree on scaled
    /// (125 %, 150 %) displays. The app window usually did this already; then this does nothing.
    pub fn dpi_aware() {
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    pub fn move_pointer(x: i32, y: i32) -> bool {
        let (vx, vy, vw, vh) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if vw <= 1 || vh <= 1 {
            return false;
        }
        // 0..65535 spans the whole virtual desktop, every monitor included.
        let nx = ((x - vx).clamp(0, vw - 1) as i64 * 65535 / (vw - 1) as i64) as i32;
        let ny = ((y - vy).clamp(0, vh - 1) as i64 * 65535 / (vh - 1) as i64) as i32;
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: nx,
                    dy: ny,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) == 1 }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use crate::PermissionKind;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    pub fn screen_allowed() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn control_allowed() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// Puts OmniDesk in the list in System Settings (its switch still has to be turned on) and
    /// opens that page.
    pub fn request(kind: PermissionKind) {
        let pane = match kind {
            PermissionKind::Screen => {
                unsafe {
                    CGRequestScreenCaptureAccess();
                }
                "Privacy_ScreenCapture"
            }
            PermissionKind::Control => "Privacy_Accessibility",
        };
        let _ = std::process::Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .spawn();
    }
}

fn device_name() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil")
            .args(["--get", "ComputerName"])
            .output()
        {
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if out.status.success() && !name.is_empty() {
                return name;
            }
        }
    }
    let host = gethostname::gethostname().to_string_lossy().to_string();
    let host = host.strip_suffix(".local").unwrap_or(&host).to_string();
    if host.is_empty() {
        "Computer".to_string()
    } else {
        host
    }
}

#[cfg(target_os = "windows")]
fn has_battery() -> bool {
    #[repr(C)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    }
    let mut status = SystemPowerStatus {
        ac_line_status: 0,
        battery_flag: 0,
        battery_life_percent: 0,
        system_status_flag: 0,
        battery_life_time: 0,
        battery_full_life_time: 0,
    };
    // battery_flag: 128 = no system battery, 255 = unknown.
    let ok = unsafe { GetSystemPowerStatus(&mut status) } != 0;
    ok && status.battery_flag != 255 && status.battery_flag & 128 == 0
}

#[cfg(target_os = "macos")]
fn has_battery() -> bool {
    std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).contains("InternalBattery"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn has_battery() -> bool {
    std::fs::read_dir("/sys/class/power_supply")
        .map(|dir| {
            dir.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        })
        .unwrap_or(false)
}
