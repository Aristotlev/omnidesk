//! Wire protocol shared by every OmniDesk device.
//!
//! Messages travel inside the encrypted link (`omnidesk-link`), encoded with postcard.
//! Never reorder or remove enum variants: append only, and bump [`PROTOCOL_VERSION`]
//! when an old build could not understand a new one.

use serde::{Deserialize, Serialize};
use std::fmt;

pub const PROTOCOL_VERSION: u16 = 1;
/// mDNS service type every device advertises on the local network.
pub const SERVICE_TYPE: &str = "_omnidesk._tcp.local.";
/// Preferred TCP port. A free port is used instead when this one is taken.
pub const DEFAULT_PORT: u16 = 47800;
/// Largest clipboard text that is synced between devices.
pub const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Stable identifier of a device: the first 8 bytes of SHA-256 over its public key, in hex.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DeviceId(pub String);

impl DeviceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// `ab12-cd34-ef56-7890`, easier to compare by eye.
    pub fn pretty(&self) -> String {
        self.0
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect::<Vec<_>>()
            .join("-")
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Desktop,
    Laptop,
    Phone,
    Tablet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    Macos,
    Linux,
    Ios,
    Android,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::Macos => "macos",
            Os::Linux => "linux",
            Os::Ios => "ios",
            Os::Android => "android",
        }
    }
    pub fn parse(s: &str) -> Option<Os> {
        Some(match s {
            "windows" => Os::Windows,
            "macos" => Os::Macos,
            "linux" => Os::Linux,
            "ios" => Os::Ios,
            "android" => Os::Android,
            _ => return None,
        })
    }
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Desktop => "desktop",
            DeviceKind::Laptop => "laptop",
            DeviceKind::Phone => "phone",
            DeviceKind::Tablet => "tablet",
        }
    }
    pub fn parse(s: &str) -> Option<DeviceKind> {
        Some(match s {
            "desktop" => DeviceKind::Desktop,
            "laptop" => DeviceKind::Laptop,
            "phone" => DeviceKind::Phone,
            "tablet" => DeviceKind::Tablet,
            _ => return None,
        })
    }
}

/// How another device can see this device's screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewCap {
    /// The screen cannot be shared (no display, or permission refused).
    None,
    /// The owner has to start sharing on the device itself each time (iPhone, iPad).
    OnDemand,
    /// Any paired device can open the screen at any time.
    Always,
}

/// What this device can do. Sent in [`Message::Hello`] so the UI only offers what works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Its screen can be watched from other devices.
    pub view: ViewCap,
    /// Other devices can move its pointer and type on it.
    pub control: bool,
    /// Its own keyboard and mouse can be handed over to a neighbouring screen (Flow).
    pub grab: bool,
    /// Can show another device's desktop as an extra monitor.
    pub display_sink: bool,
    /// Can add a virtual monitor that another device displays.
    pub display_source: bool,
    /// Shares its text clipboard.
    pub clipboard: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenInfo {
    pub index: u32,
    pub name: String,
    /// Position and size in the device's own desktop coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub primary: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub os: Os,
    pub protocol: u16,
    pub app_version: String,
    /// TCP port this device accepts connections on (the port of an incoming connection differs).
    pub listen_port: u16,
    pub capabilities: Capabilities,
    pub screens: Vec<ScreenInfo>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// Keyboard and pointer input sent to the device being controlled.
///
/// Externally tagged on purpose (`{"pointer_abs": {...}}` in JSON): postcard cannot decode
/// internally tagged enums.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputEvent {
    /// Pointer position on one screen, `x` and `y` from 0.0 (left / top) to 1.0 (right / bottom).
    PointerAbs {
        screen: u32,
        x: f32,
        y: f32,
    },
    /// Relative pointer motion in pixels (Flow, games).
    PointerRel {
        dx: i32,
        dy: i32,
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    /// Wheel steps. Positive `dy` scrolls down, positive `dx` scrolls right.
    Scroll {
        dx: i32,
        dy: i32,
    },
    /// `code` is the physical key (W3C `KeyboardEvent.code`, e.g. `KeyA`, `ShiftLeft`),
    /// `key` the character or named key it produces (W3C `KeyboardEvent.key`).
    Key {
        code: String,
        key: String,
        down: bool,
    },
    /// Text typed without key events (touch keyboards, dictation, IME).
    Text {
        text: String,
    },
}

/// Options for a live screen stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamOptions {
    /// Frames are scaled down to at most this width.
    pub max_width: u32,
    pub fps: u8,
    /// JPEG quality, 1 to 100.
    pub quality: u8,
}

impl StreamOptions {
    /// Small and slow: one tile on the wall.
    pub const THUMBNAIL: StreamOptions = StreamOptions {
        max_width: 640,
        fps: 4,
        quality: 60,
    };
    /// Full size: watching or controlling one screen.
    pub const FOCUS: StreamOptions = StreamOptions {
        max_width: 1920,
        fps: 20,
        quality: 72,
    };

    pub fn clamped(self) -> StreamOptions {
        StreamOptions {
            max_width: self.max_width.clamp(160, 3840),
            fps: self.fps.clamp(1, 30),
            quality: self.quality.clamp(20, 95),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameFormat {
    Jpeg,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub screen: u32,
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

/// Everything two devices say to each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// First message on a trusted link, and again whenever screens change.
    Hello(DeviceInfo),
    Ping(u64),
    Pong(u64),

    // Pairing. The only messages accepted from a device that is not paired yet.
    PairInit {
        #[serde(with = "serde_bytes")]
        spake: Vec<u8>,
    },
    PairReply {
        #[serde(with = "serde_bytes")]
        spake: Vec<u8>,
        confirm: [u8; 32],
    },
    PairConfirm {
        confirm: [u8; 32],
    },
    PairReject {
        reason: String,
    },

    // Screens.
    StreamStart {
        screen: u32,
        options: StreamOptions,
    },
    StreamStop {
        screen: u32,
    },
    Frame(Frame),
    StreamError {
        screen: u32,
        reason: String,
    },

    // Control.
    Input(InputEvent),

    // Shared clipboard.
    Clipboard {
        text: String,
    },

    /// The other side removed this device from its paired list.
    Unpaired,

    /// Something the person on the other device should know, such as a missing permission
    /// that stops this device from being controlled.
    Notice {
        text: String,
    },
}

impl Message {
    pub fn encode(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_stdvec(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Message, postcard::Error> {
        postcard::from_bytes(bytes)
    }
    /// Messages a device may send before it is paired.
    pub fn allowed_unpaired(&self) -> bool {
        matches!(
            self,
            Message::PairInit { .. }
                | Message::PairReply { .. }
                | Message::PairConfirm { .. }
                | Message::PairReject { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_keeps_bytes_compact() {
        let msg = Message::Frame(Frame {
            screen: 1,
            seq: 42,
            width: 640,
            height: 360,
            format: FrameFormat::Jpeg,
            data: vec![7u8; 50_000],
        });
        let bytes = msg.encode().unwrap();
        assert!(
            bytes.len() < 50_000 + 32,
            "frame payload must not be inflated: {}",
            bytes.len()
        );
        assert_eq!(Message::decode(&bytes).unwrap(), msg);
    }

    #[test]
    fn input_roundtrip() {
        for ev in [
            InputEvent::PointerAbs {
                screen: 0,
                x: 0.25,
                y: 0.75,
            },
            InputEvent::PointerRel { dx: -4, dy: 9 },
            InputEvent::Button {
                button: MouseButton::Right,
                down: true,
            },
            InputEvent::Scroll { dx: 0, dy: -3 },
            InputEvent::Key {
                code: "KeyA".into(),
                key: "a".into(),
                down: true,
            },
            InputEvent::Text {
                text: "héllo".into(),
            },
        ] {
            let msg = Message::Input(ev);
            assert_eq!(Message::decode(&msg.encode().unwrap()).unwrap(), msg);
        }
    }

    #[test]
    fn only_pairing_messages_before_trust() {
        assert!(Message::PairInit { spake: vec![1] }.allowed_unpaired());
        assert!(!Message::Clipboard { text: "x".into() }.allowed_unpaired());
        assert!(!Message::Input(InputEvent::PointerRel { dx: 1, dy: 1 }).allowed_unpaired());
        assert!(!Message::StreamStart {
            screen: 0,
            options: StreamOptions::THUMBNAIL
        }
        .allowed_unpaired());
    }

    #[test]
    fn device_id_pretty() {
        assert_eq!(
            DeviceId("ab12cd34ef567890".into()).pretty(),
            "ab12-cd34-ef56-7890"
        );
    }
}
