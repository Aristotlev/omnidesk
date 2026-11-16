//! The OmniDesk app: a window (and tray icon) around `omnidesk-node`.

use omnidesk_node::{
    native_platform, DeviceId, DeviceInfo, DeviceView, InputEvent, Node, NodeConfig, NodeEvent,
    PairingOffer, PermissionKind, Permissions, Settings, StreamOptions,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    node: Node,
}

#[derive(Serialize)]
struct SelfView {
    info: DeviceInfo,
    settings: Settings,
    /// This computer's address on the local network, for pairing by address.
    address: Option<String>,
    port: u16,
}

#[derive(Clone, Serialize)]
struct FrameEvent {
    device: DeviceId,
    screen: u32,
    seq: u64,
    width: u32,
    height: u32,
}

#[derive(Clone, Serialize)]
struct StreamErrorEvent {
    device: DeviceId,
    screen: u32,
    reason: String,
}

#[derive(Clone, Serialize)]
struct PairingEndedEvent {
    reason: String,
    code_valid: bool,
}

#[derive(Clone, Serialize)]
struct PairedEvent {
    device: DeviceId,
    name: String,
}

#[tauri::command]
fn self_info(state: State<AppState>) -> SelfView {
    SelfView {
        info: state.node.info(),
        settings: state.node.settings(),
        address: omnidesk_node::local_ip().map(|ip| ip.to_string()),
        port: state.node.port(),
    }
}

#[tauri::command]
fn devices(state: State<AppState>) -> Vec<DeviceView> {
    state.node.devices()
}

#[tauri::command]
fn begin_pairing(state: State<AppState>) -> PairingOffer {
    state.node.begin_pairing()
}

#[tauri::command]
fn cancel_pairing(state: State<AppState>) {
    state.node.cancel_pairing()
}

#[tauri::command]
async fn pair(
    state: State<'_, AppState>,
    target: String,
    code: String,
) -> Result<DeviceId, String> {
    let node = state.node.clone();
    node.pair(&target, &code).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn unpair(state: State<AppState>, device: String) -> Result<(), String> {
    state
        .node
        .unpair(&DeviceId(device))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_settings(state: State<AppState>, settings: Settings) {
    state.node.set_settings(settings)
}

/// `mode` is "focus" (full size, for one screen) or anything else for a wall thumbnail.
#[tauri::command]
fn watch(state: State<AppState>, device: String, screen: u32, mode: String) {
    let options = if mode == "focus" {
        StreamOptions::FOCUS
    } else {
        StreamOptions::THUMBNAIL
    };
    state.node.watch(&DeviceId(device), screen, options)
}

#[tauri::command]
fn unwatch(state: State<AppState>, device: String, screen: u32) {
    state.node.unwatch(&DeviceId(device), screen)
}

#[tauri::command]
fn send_input(state: State<AppState>, device: String, event: InputEvent) -> Result<(), String> {
    state
        .node
        .send_input(&DeviceId(device), event)
        .map_err(|e| e.to_string())
}

/// What macOS lets other devices do with this Mac. Always "not needed" on Windows and Linux.
#[tauri::command]
fn permissions(state: State<AppState>) -> Permissions {
    state.node.permissions()
}

#[tauri::command]
fn request_permission(state: State<AppState>, kind: PermissionKind) {
    state.node.request_permission(kind)
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Tray icon: OmniDesk keeps running there so this computer stays reachable with its window closed.
#[cfg(desktop)]
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let open = MenuItem::with_id(app, "open", "Open OmniDesk", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit OmniDesk", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("OmniDesk")
        .menu(&menu)
        .show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "open" => show_main(app),
        "quit" => app.exit(0),
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

fn forward_events(app: AppHandle, node: Node, mut events: omnidesk_node::EventReceiver) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            let _ = match event {
                NodeEvent::Devices => app.emit("node://devices", node.devices()),
                NodeEvent::Frame {
                    device,
                    screen,
                    seq,
                    width,
                    height,
                } => app.emit(
                    "node://frame",
                    FrameEvent {
                        device,
                        screen,
                        seq,
                        width,
                        height,
                    },
                ),
                NodeEvent::StreamError {
                    device,
                    screen,
                    reason,
                } => app.emit(
                    "node://stream-error",
                    StreamErrorEvent {
                        device,
                        screen,
                        reason,
                    },
                ),
                NodeEvent::Paired { device, name } => {
                    app.emit("node://paired", PairedEvent { device, name })
                }
                NodeEvent::PairingEnded { reason, code_valid } => app.emit(
                    "node://pairing-ended",
                    PairingEndedEvent { reason, code_valid },
                ),
                NodeEvent::Notice(text) => app.emit("node://notice", text),
            };
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder
        // Opening OmniDesk a second time brings back the window of the copy already running.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    let app = builder
        // Frames go to the window as plain images: `frame://localhost/<device>-<screen>`.
        .register_uri_scheme_protocol("frame", |ctx, request| {
            let path = request.uri().path().trim_start_matches('/').to_string();
            let frame = path.rsplit_once('-').and_then(|(device, screen)| {
                let screen: u32 = screen.parse().ok()?;
                ctx.app_handle()
                    .try_state::<AppState>()?
                    .node
                    .frame(&DeviceId(device.to_string()), screen)
            });
            let response = tauri::http::Response::builder()
                .header("Access-Control-Allow-Origin", "*")
                .header("Cache-Control", "no-store");
            match frame {
                Some(frame) => response
                    .status(200)
                    .header("Content-Type", "image/jpeg")
                    .body(frame.data.clone()),
                None => response.status(404).body(Vec::new()),
            }
            .expect("valid response")
        })
        .setup(|app| {
            let config = NodeConfig {
                data_dir: app.path().app_data_dir()?,
                port: 0,
                // Phones and tablets find computers through the system's own network browser (not built yet).
                mdns: cfg!(desktop),
                app_version: app.package_info().version.to_string(),
                platform: native_platform(),
            };
            let (node, events) = tauri::async_runtime::block_on(Node::start(config))?;
            app.manage(AppState { node: node.clone() });
            forward_events(app.handle().clone(), node, events);

            #[cfg(desktop)]
            if let Err(e) = build_tray(app) {
                eprintln!("tray icon unavailable: {e}");
            }
            // Started at sign-in: stay in the tray. Started by the person: show the window.
            if !std::env::args().any(|a| a == "--hidden") {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps OmniDesk running in the tray. "Quit OmniDesk" there stops it.
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
            #[cfg(not(desktop))]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            self_info,
            devices,
            begin_pairing,
            cancel_pairing,
            pair,
            unpair,
            set_settings,
            watch,
            unwatch,
            send_input,
            permissions,
            request_permission,
            app_version
        ])
        .build(tauri::generate_context!())
        .expect("error while starting OmniDesk");

    app.run(|app, event| {
        // Clicking the Dock icon on a Mac reopens the window.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            show_main(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}
