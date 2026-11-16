//! Everything that touches the operating system: screens, capture, input, clipboard.
//!
//! The rest of OmniDesk talks to the [`Platform`] trait only.
//!
//! * [`native`] returns the implementation for the OS this build runs on. Windows, macOS and
//!   Linux share one implementation built on cross-platform crates (`xcap`, `enigo`, `arboard`).
//!   iOS and Android builds get [`Viewer`], which can watch and control other devices but does
//!   not share its own screen yet.
//! * [`Synthetic`] is a fake device with a generated screen, used by tests and `--synthetic`.

use omnidesk_proto::{
    Capabilities, DeviceKind, InputEvent, Os, ScreenInfo, StreamOptions, ViewCap,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
mod desktop;
mod keys;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("this device cannot do that")]
    Unsupported,
    #[error("screen {0} does not exist")]
    NoSuchScreen(u32),
    #[error("screen capture failed: {0}")]
    Capture(String),
    #[error("input failed: {0}")]
    Input(String),
    #[error("image encoding failed: {0}")]
    Encode(String),
    #[error("{}", permission_message(*.0))]
    Permission(PermissionKind),
}

fn permission_message(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Screen => "This Mac does not allow OmniDesk to record its screen yet. On the Mac, open OmniDesk and click Allow Screen Recording.",
        PermissionKind::Control => "This Mac does not allow OmniDesk to control it yet. On the Mac, open OmniDesk and click Allow Accessibility.",
    }
}

/// One captured frame, already scaled and JPEG-encoded.
#[derive(Clone, Debug)]
pub struct EncodedFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Whether the operating system lets OmniDesk do something on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Granted,
    /// The person has to allow it in the system's privacy settings first.
    Missing,
    /// This system does not ask.
    NotNeeded,
}

impl Access {
    pub fn ok(self) -> bool {
        self != Access::Missing
    }
}

/// System permissions other devices depend on. Only macOS asks for them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Permissions {
    /// Capturing the screen (macOS: Screen Recording).
    pub screen: Access,
    /// Playing keyboard and mouse input (macOS: Accessibility).
    pub control: Access,
}

impl Permissions {
    pub const NOT_NEEDED: Permissions = Permissions {
        screen: Access::NotNeeded,
        control: Access::NotNeeded,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Screen,
    Control,
}

pub trait Platform: Send + Sync + 'static {
    fn os(&self) -> Os;
    fn kind(&self) -> DeviceKind;
    /// Name shown to other devices until the user picks one.
    fn device_name(&self) -> String;
    fn capabilities(&self) -> Capabilities;
    fn screens(&self) -> Vec<ScreenInfo>;
    /// Captures one frame of `screen`. Called repeatedly from one dedicated thread per stream.
    fn capture(&self, screen: u32, options: StreamOptions) -> Result<EncodedFrame, PlatformError>;
    /// Plays one input event on this device. Must not block.
    fn input(&self, event: InputEvent) -> Result<(), PlatformError>;
    fn clipboard_get(&self) -> Option<String>;
    fn clipboard_set(&self, text: &str);
    /// System permissions as they are right now.
    fn permissions(&self) -> Permissions {
        Permissions::NOT_NEEDED
    }
    /// Shows the system's prompt for a permission and opens the matching settings page.
    fn request_permission(&self, _kind: PermissionKind) {}
}

/// The platform implementation for the OS this build runs on.
pub fn native() -> Arc<dyn Platform> {
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
    {
        Arc::new(desktop::Desktop::new())
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Arc::new(Viewer::new())
    }
}

/// Scales an RGBA image down to `options.max_width` and encodes it as JPEG.
pub fn encode_rgba(
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    options: StreamOptions,
) -> Result<EncodedFrame, PlatformError> {
    use fast_image_resize as fir;
    let options = options.clamped();
    if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return Err(PlatformError::Encode("empty image".into()));
    }
    let (out_w, out_h, pixels) = if width > options.max_width {
        let out_w = options.max_width;
        let out_h = ((height as u64 * out_w as u64) / width as u64).max(1) as u32;
        let src = fir::images::Image::from_vec_u8(width, height, rgba, fir::PixelType::U8x4)
            .map_err(|e| PlatformError::Encode(e.to_string()))?;
        let mut dst = fir::images::Image::new(out_w, out_h, fir::PixelType::U8x4);
        let resize = fir::ResizeOptions::new()
            .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear))
            .use_alpha(false);
        fir::Resizer::new()
            .resize(&src, &mut dst, &resize)
            .map_err(|e| PlatformError::Encode(e.to_string()))?;
        (out_w, out_h, dst.into_vec())
    } else {
        (width, height, rgba)
    };
    if out_w > u16::MAX as u32 || out_h > u16::MAX as u32 {
        return Err(PlatformError::Encode("image too large".into()));
    }
    let mut data = Vec::with_capacity((out_w * out_h / 6) as usize);
    jpeg_encoder::Encoder::new(&mut data, options.quality)
        .encode(
            &pixels,
            out_w as u16,
            out_h as u16,
            jpeg_encoder::ColorType::Rgba,
        )
        .map_err(|e| PlatformError::Encode(e.to_string()))?;
    Ok(EncodedFrame {
        width: out_w,
        height: out_h,
        data,
    })
}

fn no_capabilities() -> Capabilities {
    Capabilities {
        view: ViewCap::None,
        control: false,
        grab: false,
        display_sink: false,
        display_source: false,
        clipboard: false,
    }
}

/// A device that watches and controls others but shares nothing itself (phones and tablets today).
pub struct Viewer {
    os: Os,
    kind: DeviceKind,
    name: String,
}

impl Viewer {
    pub fn new() -> Viewer {
        let (os, kind, name) = if cfg!(target_os = "ios") {
            (Os::Ios, DeviceKind::Phone, "iPhone or iPad")
        } else if cfg!(target_os = "android") {
            (Os::Android, DeviceKind::Phone, "Android device")
        } else {
            (Os::Linux, DeviceKind::Desktop, "Viewer")
        };
        Viewer {
            os,
            kind,
            name: name.to_string(),
        }
    }
    pub fn with(os: Os, kind: DeviceKind, name: &str) -> Viewer {
        Viewer {
            os,
            kind,
            name: name.to_string(),
        }
    }
}

impl Default for Viewer {
    fn default() -> Self {
        Viewer::new()
    }
}

impl Platform for Viewer {
    fn os(&self) -> Os {
        self.os
    }
    fn kind(&self) -> DeviceKind {
        self.kind
    }
    fn device_name(&self) -> String {
        self.name.clone()
    }
    fn capabilities(&self) -> Capabilities {
        no_capabilities()
    }
    fn screens(&self) -> Vec<ScreenInfo> {
        Vec::new()
    }
    fn capture(
        &self,
        _screen: u32,
        _options: StreamOptions,
    ) -> Result<EncodedFrame, PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn input(&self, _event: InputEvent) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
    fn clipboard_get(&self) -> Option<String> {
        None
    }
    fn clipboard_set(&self, _text: &str) {}
}

/// A fake computer: one generated 1280x720 screen, a recorded input log and an in-memory
/// clipboard. Lets the whole pipeline run without a display.
pub struct Synthetic {
    name: String,
    os: Os,
    kind: DeviceKind,
    frames: AtomicU64,
    inputs: Mutex<Vec<InputEvent>>,
    clipboard: Mutex<Option<String>>,
    /// Acts like a Mac that has not allowed control yet.
    deny_control: std::sync::atomic::AtomicBool,
}

impl Synthetic {
    pub const WIDTH: u32 = 1280;
    pub const HEIGHT: u32 = 720;

    pub fn new(name: &str) -> Synthetic {
        Synthetic {
            name: name.to_string(),
            os: Os::Linux,
            kind: DeviceKind::Desktop,
            frames: AtomicU64::new(0),
            inputs: Mutex::new(Vec::new()),
            clipboard: Mutex::new(None),
            deny_control: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub fn with_kind(mut self, os: Os, kind: DeviceKind) -> Synthetic {
        self.os = os;
        self.kind = kind;
        self
    }
    /// Makes input fail the way it does on a Mac without the Accessibility permission.
    pub fn deny_control(&self, deny: bool) {
        self.deny_control.store(deny, Ordering::Relaxed);
    }
    /// Every input event this device has been asked to play.
    pub fn inputs(&self) -> Vec<InputEvent> {
        self.inputs.lock().unwrap().clone()
    }
    pub fn frames_captured(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

impl Platform for Synthetic {
    fn os(&self) -> Os {
        self.os
    }
    fn kind(&self) -> DeviceKind {
        self.kind
    }
    fn device_name(&self) -> String {
        self.name.clone()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            view: ViewCap::Always,
            control: true,
            grab: false,
            display_sink: false,
            display_source: false,
            clipboard: true,
        }
    }
    fn screens(&self) -> Vec<ScreenInfo> {
        vec![ScreenInfo {
            index: 0,
            name: "Test screen".into(),
            x: 0,
            y: 0,
            width: Self::WIDTH,
            height: Self::HEIGHT,
            scale: 1.0,
            primary: true,
        }]
    }
    fn capture(&self, screen: u32, options: StreamOptions) -> Result<EncodedFrame, PlatformError> {
        if screen != 0 {
            return Err(PlatformError::NoSuchScreen(screen));
        }
        let n = self.frames.fetch_add(1, Ordering::Relaxed);
        let (w, h) = (Self::WIDTH, Self::HEIGHT);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        let bar = ((n * 16) % w as u64) as u32;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let on_bar = x >= bar && x < bar + 24;
                rgba[i] = if on_bar { 255 } else { (x * 255 / w) as u8 };
                rgba[i + 1] = if on_bar { 255 } else { (y * 255 / h) as u8 };
                rgba[i + 2] = if on_bar { 255 } else { 96 };
                rgba[i + 3] = 255;
            }
        }
        encode_rgba(rgba, w, h, options)
    }
    fn input(&self, event: InputEvent) -> Result<(), PlatformError> {
        if self.deny_control.load(Ordering::Relaxed) {
            return Err(PlatformError::Permission(PermissionKind::Control));
        }
        self.inputs.lock().unwrap().push(event);
        Ok(())
    }
    fn clipboard_get(&self) -> Option<String> {
        self.clipboard.lock().unwrap().clone()
    }
    fn clipboard_set(&self, text: &str) {
        *self.clipboard.lock().unwrap() = Some(text.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_jpeg(data: &[u8]) -> bool {
        data.len() > 4 && data[..2] == [0xFF, 0xD8] && data[data.len() - 2..] == [0xFF, 0xD9]
    }

    #[test]
    fn synthetic_frames_are_scaled_jpegs() {
        let dev = Synthetic::new("test");
        let thumb = dev.capture(0, StreamOptions::THUMBNAIL).unwrap();
        assert_eq!((thumb.width, thumb.height), (640, 360));
        assert!(is_jpeg(&thumb.data));
        let full = dev.capture(0, StreamOptions::FOCUS).unwrap();
        assert_eq!((full.width, full.height), (1280, 720));
        assert!(is_jpeg(&full.data));
        assert!(full.data.len() > thumb.data.len());
        assert!(matches!(
            dev.capture(3, StreamOptions::FOCUS),
            Err(PlatformError::NoSuchScreen(3))
        ));
    }

    #[test]
    fn encode_rejects_short_buffers() {
        assert!(encode_rgba(vec![0; 10], 100, 100, StreamOptions::FOCUS).is_err());
    }

    #[test]
    fn viewer_shares_nothing() {
        let v = Viewer::with(Os::Ios, DeviceKind::Tablet, "iPad");
        assert_eq!(v.capabilities().view, ViewCap::None);
        assert!(v.screens().is_empty());
        assert!(v.input(InputEvent::PointerRel { dx: 1, dy: 1 }).is_err());
    }
}
