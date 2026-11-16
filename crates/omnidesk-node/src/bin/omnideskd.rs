//! OmniDesk without a window: runs the node in a terminal.
//!
//! Useful on a headless machine (it can still be watched and controlled), for scripting, and
//! for testing two devices on one computer. Type `help` once it is running.

use clap::Parser;
use omnidesk_node::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(name = "omnideskd", version, about = "OmniDesk node for the terminal")]
struct Args {
    /// Folder for this device's key, paired devices and settings
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// TCP port to listen on (default: 47800, or any free port when taken)
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Name shown to other devices
    #[arg(long)]
    name: Option<String>,
    /// Do not announce or search on the local network
    #[arg(long)]
    no_mdns: bool,
    /// Share a generated test screen instead of the real one
    #[arg(long)]
    synthetic: bool,
}

fn default_data_dir() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("omnideskd")
}

fn find(node: &Node, prefix: &str) -> Option<DeviceId> {
    let matches: Vec<DeviceView> = node
        .devices()
        .into_iter()
        .filter(|d| d.id.as_str().starts_with(prefix) || d.name.eq_ignore_ascii_case(prefix))
        .collect();
    match matches.as_slice() {
        [one] => Some(one.id.clone()),
        [] => {
            println!("no device matches \"{prefix}\"");
            None
        }
        _ => {
            println!("more than one device matches \"{prefix}\"");
            None
        }
    }
}

fn print_devices(node: &Node) {
    let list = node.devices();
    if list.is_empty() {
        println!("no devices yet");
    }
    for d in list {
        let state = match (d.paired, d.online) {
            (true, true) => "paired, online",
            (true, false) => "paired, offline",
            _ => "nearby, not paired",
        };
        println!(
            "{}  {:<24} {:<8} {:<8} {:<18} {} screen(s)  {}",
            d.id.pretty(),
            d.name,
            d.os.as_str(),
            d.kind.as_str(),
            state,
            d.screens.len(),
            d.address.unwrap_or_default()
        );
    }
}

const HELP: &str = "\
devices                      list paired and nearby devices
code                         show a pairing code on this device
pair <device|address> <code> pair with the device showing <code>
unpair <device>              forget a paired device
watch <device> [screen]      start receiving its screen
save <device> <file> [screen] write the latest received frame to a .jpg file
move <device> <x> <y>        move its pointer (0..1 across the screen)
click <device>               left click
type <device> <text>         type text on it
name <new name>              rename this device
quit";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    let args = Args::parse();
    let platform: Arc<dyn Platform> = if args.synthetic {
        Arc::new(Synthetic::new("Test device"))
    } else {
        native_platform()
    };
    let config = NodeConfig {
        data_dir: args.data_dir.unwrap_or_else(default_data_dir),
        port: args.port,
        mdns: !args.no_mdns,
        app_version: env!("CARGO_PKG_VERSION").into(),
        platform,
    };
    let (node, mut events) = Node::start(config).await?;
    if let Some(name) = args.name {
        node.set_settings(Settings {
            name: Some(name),
            ..node.settings()
        });
    }
    let me = node.info();
    println!(
        "{} ({}) listening on port {}, {} screen(s)",
        me.name,
        me.id.pretty(),
        node.port(),
        me.screens.len()
    );

    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                NodeEvent::Devices | NodeEvent::Frame { .. } => {}
                NodeEvent::Paired { name, device } => {
                    println!("paired with {name} ({})", device.pretty())
                }
                NodeEvent::PairingEnded { reason, .. } => println!("pairing: {reason}"),
                NodeEvent::StreamError { reason, .. } => println!("stream: {reason}"),
                NodeEvent::Notice(text) => println!("notice: {text}"),
            }
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else {
            continue;
        };
        let rest: Vec<&str> = words.collect();
        match (command, rest.as_slice()) {
            ("help", _) => println!("{HELP}"),
            ("devices", _) => print_devices(&node),
            ("code", _) => {
                let offer = node.begin_pairing();
                println!(
                    "pairing code: {} {} (valid for {} seconds)",
                    &offer.code[..3],
                    &offer.code[3..],
                    offer.seconds
                );
            }
            ("pair", [target, code @ ..]) if !code.is_empty() => {
                let target = find_quiet(&node, target)
                    .map(|id| id.0)
                    .unwrap_or_else(|| target.to_string());
                match node.pair(&target, &code.concat()).await {
                    Ok(id) => println!("paired with {}", id.pretty()),
                    Err(e) => println!("pairing failed: {e}"),
                }
            }
            ("unpair", [device]) => {
                if let Some(id) = find(&node, device) {
                    match node.unpair(&id) {
                        Ok(()) => println!("forgotten"),
                        Err(e) => println!("{e}"),
                    }
                }
            }
            ("watch", [device, screen @ ..]) => {
                if let Some(id) = find(&node, device) {
                    node.watch(
                        &id,
                        screen.first().and_then(|s| s.parse().ok()).unwrap_or(0),
                        StreamOptions::FOCUS,
                    );
                    println!("watching");
                }
            }
            ("save", [device, file, screen @ ..]) => {
                if let Some(id) = find(&node, device) {
                    match node.frame(
                        &id,
                        screen.first().and_then(|s| s.parse().ok()).unwrap_or(0),
                    ) {
                        Some(frame) => {
                            std::fs::write(file, &frame.data)?;
                            println!(
                                "saved {}x{} frame {} to {file}",
                                frame.width, frame.height, frame.seq
                            );
                        }
                        None => println!("no frame yet. Use: watch {device}"),
                    }
                }
            }
            ("move", [device, x, y]) => {
                if let (Some(id), Ok(x), Ok(y)) = (find(&node, device), x.parse(), y.parse()) {
                    report(node.send_input(&id, InputEvent::PointerAbs { screen: 0, x, y }));
                }
            }
            ("click", [device]) => {
                if let Some(id) = find(&node, device) {
                    let _ = node.send_input(
                        &id,
                        InputEvent::Button {
                            button: MouseButton::Left,
                            down: true,
                        },
                    );
                    report(node.send_input(
                        &id,
                        InputEvent::Button {
                            button: MouseButton::Left,
                            down: false,
                        },
                    ));
                }
            }
            ("type", [device, text @ ..]) if !text.is_empty() => {
                if let Some(id) = find(&node, device) {
                    report(node.send_input(
                        &id,
                        InputEvent::Text {
                            text: text.join(" "),
                        },
                    ));
                }
            }
            ("name", words) if !words.is_empty() => {
                node.set_settings(Settings {
                    name: Some(words.join(" ")),
                    ..node.settings()
                });
                println!("renamed");
            }
            ("quit" | "exit", _) => break,
            _ => println!("unknown command. Type: help"),
        }
    }
    node.shutdown();
    Ok(())
}

fn find_quiet(node: &Node, prefix: &str) -> Option<DeviceId> {
    let mut matches = node
        .devices()
        .into_iter()
        .filter(|d| d.id.as_str().starts_with(prefix) || d.name.eq_ignore_ascii_case(prefix));
    match (matches.next(), matches.next()) {
        (Some(one), None) => Some(one.id),
        _ => None,
    }
}

fn report(result: Result<(), NodeError>) {
    match result {
        Ok(()) => println!("sent"),
        Err(e) => println!("{e}"),
    }
}
