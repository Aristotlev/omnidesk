# OmniDesk architecture

Goal: every device you own on one desk. See all screens live, control any of them from the one you are at, move one keyboard and mouse across all of them, and share the clipboard.

## What each platform allows

| | See its screen | Control it | Use it to control others | Extra monitor for a computer |
|---|---|---|---|---|
| **Windows** | Yes | Yes | Yes | Yes (virtual display driver, phase 3) |
| **macOS** | Yes (Screen Recording permission) | Yes (Accessibility permission) | Yes | Yes (virtual display, phase 3) |
| **Linux X11** | Yes | Yes | Yes | Phase 3 |
| **Linux Wayland** | Phase 2 (ScreenCast portal) | Phase 2 (RemoteDesktop portal / libei) | Yes | Phase 3 |
| **Android** | Phase 4 (MediaProjection, user consent each session) | Phase 4 (Accessibility service) | Phase 2 | Phase 3 |
| **iPhone / iPad** | Only while the owner taps *Start sharing* (ReplayKit broadcast, phase 4) | **No.** Apple allows no app to inject touches. Only path: a Bluetooth HID accessory (phase 5) | Phase 2 | Yes, as a display for a computer (phase 3) |

Verified constraints (November 2026):

- No third-party app can remote-control an iPhone or iPad. RustDesk documents the same limit. iOS screen capture only exists as a user-started ReplayKit broadcast extension with a 50 MB memory cap.
- Apple's own iPhone Mirroring is not available in the EU (iOS 26 / macOS 26), so there is no Apple route either.
- An iPhone does accept a Bluetooth mouse through AssistiveTouch. A small BLE HID accessory (ESP32-S3 class, a few euros) driven by OmniDesk can tap and swipe on an iPhone. Combined with a ReplayKit broadcast, that is real iPhone control. This is a hardware add-on and a possible product of its own.

## Components

```
            ┌──────────────── one device ─────────────────┐
 React UI ──┤ src-tauri: window, tray, autostart, updater │
            │        │ commands + events, frame://        │
            │  omnidesk-node                              │
            │   ├─ sessions (one per paired device)       │
            │   ├─ streams: capture → scale → JPEG        │
            │   ├─ input playback, clipboard sync         │
            │   └─ maintenance: reconnect, screen changes │
            │  omnidesk-link     Noise XX + SPAKE2 pairing│
            │  omnidesk-discovery  mDNS _omnidesk._tcp    │
            │  omnidesk-platform   xcap / enigo / arboard │
            │  omnidesk-flow       edge crossing (KVM)    │
            └──────────────────────┬──────────────────────┘
                                   │ TCP 47800, encrypted
                              other devices
```

- **One codebase, five platforms.** Rust core plus Tauri 2. The same build becomes a Windows .exe, a Mac .dmg, a Linux AppImage, and (phase 2) iOS and Android apps. Platform code lives in `omnidesk-platform` only.
- **Peer to peer, no servers.** Devices connect to each other directly. Nothing to host, nothing to pay for, nothing that sees user data.
- **Identity.** Each device creates an X25519 key on first run. Its device id is the first 8 bytes of SHA-256 of the public key.
- **Encryption.** Every connection runs `Noise_XX_25519_ChaChaPoly_BLAKE2s`. Both sides prove their long-term key. Messages larger than 64 KiB are split into encrypted records.
- **Pairing.** One device shows a 6-digit code, the other types it. SPAKE2 over the code, bound to both long-term keys and to the Noise handshake hash, so an attacker gets one guess per code (a code is thrown away after 3 wrong tries or 2 minutes) and a relay in the middle fails even with the right code.
- **Trust.** A device that is not paired can only send pairing messages. Everything else gets "not paired" and a hang-up. Removing a device removes it on both sides.
- **Streams.** v1 sends JPEG frames: 640 px wide at 4 fps for wall tiles, up to 1920 px at 20 fps for the full-size view. Unchanged screens are not re-sent, and a slow link drops old frames instead of queueing them. Simple, works through any network, every platform decodes it.
- **Input.** W3C key names (`KeyboardEvent.code` and `.key`) on the wire, mapped to OS keys on the receiving side. Keys and buttons still held when a link drops are released.

## Phases

1. **Now (v0.1).** Windows, macOS, Linux X11. Discover, pair, live wall of every screen, full control, shared clipboard, tray, start at sign-in, auto-update, CI releases.
2. **Video and reach.** Hardware H.264/HEVC (Media Foundation, VideoToolbox, VAAPI) over WebRTC (`str0m`) for 60 fps at a fraction of the bandwidth. Remote access away from home through a relay (iroh or TURN). Wayland via portals. iOS and Android apps as viewers and controllers (Tauri mobile).
3. **One computer.** Flow: move the pointer off the edge of one screen onto the next device, with a drag-to-arrange layout editor (`omnidesk-flow` is built and tested). Tablet or laptop as an extra monitor: virtual display on the source computer (IddCx driver on Windows, CGVirtualDisplay on macOS) streamed to the other device. File drop between devices.
4. **Phones as sources.** Android full control. iPhone and iPad screen on the wall through the ReplayKit broadcast extension.
5. **OmniDesk Key.** BLE HID accessory for iPhone control.

## Store and signing

- Windows and Mac builds are not code-signed yet (same as OmniStore). Users click through one warning. A Developer ID certificate (USD 99/year) and notarization remove the Mac warning and keep macOS permissions across updates. An Authenticode certificate removes the Windows one.
- iPhone and iPad apps can only be installed from the App Store or TestFlight: Apple Developer Program membership required.
