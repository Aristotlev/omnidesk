use serde::{Deserialize, Serialize};
use std::path::Path;

/// What the person at this device has chosen. Saved as `settings.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Name shown to other devices. `None` uses the computer's own name.
    pub name: Option<String>,
    /// Paired devices may watch this screen.
    pub allow_view: bool,
    /// Paired devices may use this keyboard and mouse.
    pub allow_control: bool,
    /// Text copied on one device can be pasted on the others.
    pub share_clipboard: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: None,
            allow_view: true,
            allow_control: true,
            share_clipboard: true,
        }
    }
}

impl Settings {
    pub fn load(dir: &Path) -> Settings {
        std::fs::read(dir.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            if let Err(e) = std::fs::write(dir.join("settings.json"), json) {
                tracing::warn!("could not save settings: {e}");
            }
        }
    }
}
