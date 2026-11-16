//! The OmniDesk node: everything one device does on the network.
//!
//! A [`Node`] keeps this device's identity and paired devices, announces itself and finds
//! others on the local network, keeps an encrypted link to every paired device that is
//! reachable, and over those links shares screens, plays remote input and syncs the clipboard.
//!
//! The desktop and mobile apps (`src-tauri`) and the `omnideskd` command-line program are thin
//! shells around this crate.

mod session;
mod settings;

pub use omnidesk_link::LinkError;
pub use omnidesk_platform::{
    native as native_platform, Access, PermissionKind, Permissions, Platform, Synthetic, Viewer,
};
pub use omnidesk_proto::*;
pub use settings::Settings;

use omnidesk_discovery::{Advert, Discovery, Event as DiscoveryEvent, Found};
use omnidesk_link::{pairing, Identity, TrustStore};
use serde::Serialize;
use session::{Conn, PairRequest};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

/// How long a pairing code stays valid.
pub const PAIRING_SECONDS: u64 = 120;
/// Wrong codes accepted before a pairing code is thrown away.
const PAIRING_ATTEMPTS: u8 = 3;
const DIAL_TIMEOUT: Duration = Duration::from_secs(4);
const MAINTENANCE_EVERY: Duration = Duration::from_secs(3);
const CLIPBOARD_EVERY: Duration = Duration::from_millis(600);

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("that device was not found on this network")]
    NotFound,
    #[error("that device is not connected right now")]
    Offline,
    #[error("enter the 6-digit code shown on the other device")]
    BadCode,
    #[error("the code does not match. Check the code on the other device and try again")]
    WrongCode,
    #[error(
        "the other device is not showing a pairing code. Choose \"Show pairing code\" there first"
    )]
    NotAccepting,
    #[error("could not reach the device at {0}")]
    Unreachable(String),
    #[error("pairing was interrupted")]
    Interrupted,
    #[error("{0}")]
    Link(#[from] LinkError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub struct NodeConfig {
    /// Folder for the device key, paired devices and settings.
    pub data_dir: PathBuf,
    /// TCP port to listen on. `0` picks [`DEFAULT_PORT`] when free, otherwise any free port.
    pub port: u16,
    /// Announce and search on the local network.
    pub mdns: bool,
    pub app_version: String,
    pub platform: Arc<dyn Platform>,
}

/// One device as the UI shows it: paired, nearby, or both.
#[derive(Clone, Debug, Serialize)]
pub struct DeviceView {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub os: Os,
    pub paired: bool,
    /// Paired and connected right now.
    pub online: bool,
    /// Announced on this network.
    pub nearby: bool,
    pub address: Option<String>,
    pub capabilities: Option<Capabilities>,
    pub screens: Vec<ScreenInfo>,
    pub app_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PairingOffer {
    pub code: String,
    pub seconds: u64,
}

#[derive(Clone, Debug)]
pub enum NodeEvent {
    /// The list from [`Node::devices`] changed.
    Devices,
    /// A new frame is ready in [`Node::frame`].
    Frame {
        device: DeviceId,
        screen: u32,
        seq: u64,
        width: u32,
        height: u32,
    },
    /// A stream this device asked for cannot be delivered.
    StreamError {
        device: DeviceId,
        screen: u32,
        reason: String,
    },
    /// Another device paired with this one using the code shown here.
    Paired { device: DeviceId, name: String },
    /// Someone typed a wrong code, or the code ran out. `code_valid` tells whether the code
    /// on screen can still be used.
    PairingEnded { reason: String, code_valid: bool },
    /// Something the person at this device should know (missing OS permission, for example).
    Notice(String),
}

struct PairingWindow {
    code: String,
    expires: Instant,
    attempts_left: u8,
}

pub(crate) struct State {
    pub(crate) trust: TrustStore,
    pub(crate) settings: Settings,
    pub(crate) nearby: HashMap<DeviceId, Found>,
    pub(crate) conns: HashMap<DeviceId, Conn>,
    pub(crate) infos: HashMap<DeviceId, DeviceInfo>,
    pairing: Option<PairingWindow>,
    pub(crate) frames: HashMap<(DeviceId, u32), Arc<Frame>>,
    /// Streams this device wants, asked for again whenever the other device reconnects.
    pub(crate) watching: HashMap<(DeviceId, u32), StreamOptions>,
    dialing: HashSet<DeviceId>,
    pub(crate) screens: Vec<ScreenInfo>,
    /// Hash of the clipboard text last seen or applied, to tell a new copy from an echo.
    pub(crate) clipboard_seen: Option<u64>,
    discovery: Option<Discovery>,
    tasks: Vec<tokio::task::AbortHandle>,
    pub(crate) notices_sent: HashSet<String>,
}

pub(crate) struct Inner {
    pub(crate) identity: Identity,
    pub(crate) id: DeviceId,
    pub(crate) platform: Arc<dyn Platform>,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) state: Mutex<State>,
    pub(crate) events: mpsc::UnboundedSender<NodeEvent>,
    pub(crate) port: u16,
    app_version: String,
    data_dir: PathBuf,
    mdns: bool,
    pub(crate) serial: AtomicU64,
}

/// Stream of [`NodeEvent`]s returned by [`Node::start`].
pub type EventReceiver = mpsc::UnboundedReceiver<NodeEvent>;

/// Handle to the running node. Cheap to clone.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

pub(crate) fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

impl Inner {
    pub(crate) fn st(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn emit(&self, event: NodeEvent) {
        let _ = self.events.send(event);
    }

    /// Shows a notice once per run, however often its cause repeats.
    pub(crate) fn notice_once(&self, text: &str) {
        if self.st().notices_sent.insert(text.to_string()) {
            self.emit(NodeEvent::Notice(text.to_string()));
        }
    }

    pub(crate) fn name(&self, st: &State) -> String {
        st.settings
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| self.platform.device_name())
    }

    /// This device as other devices see it, with switched-off sharing removed.
    pub(crate) fn info(&self, st: &State) -> DeviceInfo {
        let mut capabilities = self.platform.capabilities();
        if !st.settings.allow_view {
            capabilities.view = ViewCap::None;
        }
        capabilities.control &= st.settings.allow_control;
        capabilities.clipboard &= st.settings.share_clipboard;
        DeviceInfo {
            id: self.id.clone(),
            name: self.name(st),
            kind: self.platform.kind(),
            os: self.platform.os(),
            protocol: PROTOCOL_VERSION,
            app_version: self.app_version.clone(),
            listen_port: self.port,
            capabilities,
            screens: if st.settings.allow_view {
                st.screens.clone()
            } else {
                Vec::new()
            },
        }
    }

    pub(crate) fn broadcast(&self, st: &State, message: &Message) {
        if let Ok(bytes) = message.encode() {
            for conn in st.conns.values() {
                let _ = conn.out.send(bytes.clone());
            }
        }
    }

    pub(crate) fn send_to(&self, st: &State, device: &DeviceId, message: &Message) -> bool {
        match (st.conns.get(device), message.encode()) {
            (Some(conn), Ok(bytes)) => conn.out.send(bytes).is_ok(),
            _ => false,
        }
    }

    /// Takes the pairing code for one attempt. `None` when this device is not showing a code.
    pub(crate) fn pairing_code(&self) -> Option<String> {
        let mut st = self.st();
        match &st.pairing {
            Some(w) if w.expires > Instant::now() => Some(w.code.clone()),
            _ => {
                st.pairing = None;
                None
            }
        }
    }

    /// Records the outcome of an attempt against the code shown on this device.
    pub(crate) fn pairing_result(&self, success: bool) {
        let mut st = self.st();
        let ended = match st.pairing.as_mut() {
            Some(_) if success => true,
            Some(w) => {
                w.attempts_left = w.attempts_left.saturating_sub(1);
                w.attempts_left == 0
            }
            None => false,
        };
        if ended {
            st.pairing = None;
        }
        drop(st);
        if !success {
            let reason = if ended {
                "Too many wrong codes. Show a new code to try again."
            } else {
                "A wrong code was entered on the other device."
            };
            self.emit(NodeEvent::PairingEnded {
                reason: reason.to_string(),
                code_valid: !ended,
            });
        }
    }

    fn start_discovery(self: &Arc<Self>) {
        if !self.mdns {
            return;
        }
        let advert = {
            let st = self.st();
            Advert {
                id: self.id.clone(),
                name: self.name(&st),
                kind: self.platform.kind(),
                os: self.platform.os(),
                port: self.port,
            }
        };
        // Stop the previous announcement before starting the new one.
        let previous = self.st().discovery.take();
        drop(previous);
        match Discovery::start(advert) {
            Ok((discovery, mut events)) => {
                self.st().discovery = Some(discovery);
                let inner = self.clone();
                let task = self.rt.spawn(async move {
                    while let Some(event) = events.recv().await {
                        match event {
                            DiscoveryEvent::Found(found) => {
                                let dial = {
                                    let mut st = inner.st();
                                    let id = found.id.clone();
                                    st.nearby.insert(id.clone(), found.clone());
                                    st.trust.get(&id).is_some() && !st.conns.contains_key(&id)
                                };
                                inner.emit(NodeEvent::Devices);
                                if dial {
                                    inner.dial(found.id, found.addrs);
                                }
                            }
                            DiscoveryEvent::Lost(id) => {
                                inner.st().nearby.remove(&id);
                                inner.emit(NodeEvent::Devices);
                            }
                        }
                    }
                });
                self.st().tasks.push(task.abort_handle());
            }
            Err(e) => {
                tracing::warn!("{e}");
                self.emit(NodeEvent::Notice("Devices on this network cannot be found automatically. Add them by address instead.".into()));
            }
        }
    }

    /// Connects to a paired device in the background, trying each address in turn.
    pub(crate) fn dial(self: &Arc<Self>, id: DeviceId, addrs: Vec<SocketAddr>) {
        if addrs.is_empty() || !self.st().dialing.insert(id.clone()) {
            return;
        }
        let inner = self.clone();
        self.rt.spawn(async move {
            for addr in addrs {
                if let Ok(Ok(stream)) =
                    tokio::time::timeout(DIAL_TIMEOUT, TcpStream::connect(addr)).await
                {
                    session::run(inner.clone(), stream, addr, true, None).await;
                    break;
                }
            }
            inner.st().dialing.remove(&id);
        });
    }

    fn addresses_for(&self, st: &State, id: &DeviceId) -> Vec<SocketAddr> {
        let mut addrs = st
            .nearby
            .get(id)
            .map(|f| f.addrs.clone())
            .unwrap_or_default();
        if let Some(last) = st
            .trust
            .get(id)
            .and_then(|p| p.last_addr.as_ref())
            .and_then(|a| a.parse().ok())
        {
            if !addrs.contains(&last) {
                addrs.push(last);
            }
        }
        addrs
    }

    async fn maintenance(self: Arc<Self>) {
        let mut tick = tokio::time::interval(MAINTENANCE_EVERY);
        let mut round = 0u64;
        loop {
            tick.tick().await;
            round += 1;

            // Reconnect paired devices.
            let to_dial: Vec<(DeviceId, Vec<SocketAddr>)> = {
                let st = self.st();
                st.trust
                    .list()
                    .into_iter()
                    .filter(|p| !st.conns.contains_key(&p.id) && !st.dialing.contains(&p.id))
                    .map(|p| (p.id.clone(), self.addresses_for(&st, &p.id)))
                    .collect()
            };
            for (id, addrs) in to_dial {
                self.dial(id, addrs);
            }

            // Expire the pairing code.
            let expired = {
                let mut st = self.st();
                let expired = st
                    .pairing
                    .as_ref()
                    .is_some_and(|w| w.expires <= Instant::now());
                if expired {
                    st.pairing = None;
                }
                expired
            };
            if expired {
                self.emit(NodeEvent::PairingEnded {
                    reason: "The pairing code ran out.".into(),
                    code_valid: false,
                });
            }

            // Notice monitors being plugged in, removed or rearranged.
            if round.is_multiple_of(2) {
                let platform = self.platform.clone();
                if let Ok(screens) = tokio::task::spawn_blocking(move || platform.screens()).await {
                    let mut st = self.st();
                    if st.screens != screens {
                        st.screens = screens;
                        let hello = Message::Hello(self.info(&st));
                        self.broadcast(&st, &hello);
                        drop(st);
                        self.emit(NodeEvent::Devices);
                    }
                }
            }
        }
    }

    async fn clipboard_sync(self: Arc<Self>) {
        let mut tick = tokio::time::interval(CLIPBOARD_EVERY);
        // The first look only records what is already there: connecting never pushes an old
        // clipboard to the other devices, only what is copied afterwards.
        let mut primed = false;
        loop {
            tick.tick().await;
            if !self.st().settings.share_clipboard {
                primed = false;
                continue;
            }
            let platform = self.platform.clone();
            let text = tokio::task::spawn_blocking(move || platform.clipboard_get())
                .await
                .ok()
                .flatten();
            let first_look = !primed;
            primed = true;
            let Some(text) = text else { continue };
            let hash = text_hash(&text);
            let mut st = self.st();
            if st.clipboard_seen == Some(hash) {
                continue;
            }
            st.clipboard_seen = Some(hash);
            if !first_look && !text.is_empty() && text.len() <= MAX_CLIPBOARD_BYTES {
                self.broadcast(&st, &Message::Clipboard { text });
            }
        }
    }
}

impl Node {
    /// Starts the node. Must be called inside a Tokio runtime.
    pub async fn start(
        config: NodeConfig,
    ) -> Result<(Node, mpsc::UnboundedReceiver<NodeEvent>), NodeError> {
        std::fs::create_dir_all(&config.data_dir)?;
        let identity = Identity::load_or_create(&config.data_dir)?;
        let trust = TrustStore::load(&config.data_dir)?;
        let settings = Settings::load(&config.data_dir);

        let listener = match config.port {
            0 => match TcpListener::bind(("0.0.0.0", DEFAULT_PORT)).await {
                Ok(l) => l,
                Err(_) => TcpListener::bind(("0.0.0.0", 0)).await?,
            },
            port => TcpListener::bind(("0.0.0.0", port)).await?,
        };
        let port = listener.local_addr()?.port();

        let platform = config.platform.clone();
        let screens = tokio::task::spawn_blocking(move || platform.screens())
            .await
            .unwrap_or_default();

        let (events, events_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            id: identity.id(),
            identity,
            platform: config.platform,
            rt: tokio::runtime::Handle::current(),
            state: Mutex::new(State {
                trust,
                settings,
                nearby: HashMap::new(),
                conns: HashMap::new(),
                infos: HashMap::new(),
                pairing: None,
                frames: HashMap::new(),
                watching: HashMap::new(),
                dialing: HashSet::new(),
                screens,
                clipboard_seen: None,
                discovery: None,
                tasks: Vec::new(),
                notices_sent: HashSet::new(),
            }),
            events,
            port,
            app_version: config.app_version,
            data_dir: config.data_dir,
            mdns: config.mdns,
            serial: AtomicU64::new(1),
        });

        let accept = {
            let inner = inner.clone();
            tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((stream, addr)) => {
                            tokio::spawn(session::run(inner.clone(), stream, addr, false, None));
                        }
                        Err(e) => {
                            tracing::warn!("accept failed: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
            })
        };
        let maintenance = tokio::spawn(inner.clone().maintenance());
        let clipboard = tokio::spawn(inner.clone().clipboard_sync());
        inner.st().tasks.extend([
            accept.abort_handle(),
            maintenance.abort_handle(),
            clipboard.abort_handle(),
        ]);
        inner.start_discovery();

        Ok((Node { inner }, events_rx))
    }

    /// Stops listening, announcing and every connection.
    pub fn shutdown(&self) {
        let mut st = self.inner.st();
        for task in st.tasks.drain(..) {
            task.abort();
        }
        st.discovery = None;
        st.conns.clear();
    }

    pub fn id(&self) -> DeviceId {
        self.inner.id.clone()
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn info(&self) -> DeviceInfo {
        self.inner.info(&self.inner.st())
    }

    pub fn settings(&self) -> Settings {
        self.inner.st().settings.clone()
    }

    /// Saves new settings and tells paired devices what changed.
    pub fn set_settings(&self, settings: Settings) {
        let renamed = {
            let mut st = self.inner.st();
            let renamed = st.settings.name != settings.name;
            if !settings.allow_view {
                for conn in st.conns.values_mut() {
                    conn.stop_streams();
                }
            }
            st.settings = settings;
            st.settings.save(&self.inner.data_dir);
            let hello = Message::Hello(self.inner.info(&st));
            self.inner.broadcast(&st, &hello);
            renamed
        };
        if renamed {
            self.inner.start_discovery();
        }
        self.inner.emit(NodeEvent::Devices);
    }

    /// Paired devices first (by name), then unpaired devices seen on the network.
    pub fn devices(&self) -> Vec<DeviceView> {
        let st = self.inner.st();
        let mut list: Vec<DeviceView> = st
            .trust
            .list()
            .into_iter()
            .map(|p| {
                let info = st.infos.get(&p.id).filter(|_| st.conns.contains_key(&p.id));
                DeviceView {
                    name: info.map(|i| i.name.clone()).unwrap_or(p.name),
                    kind: info.map(|i| i.kind).unwrap_or(p.kind),
                    os: info.map(|i| i.os).unwrap_or(p.os),
                    paired: true,
                    online: st.conns.contains_key(&p.id),
                    nearby: st.nearby.contains_key(&p.id),
                    address: st
                        .conns
                        .get(&p.id)
                        .map(|c| c.addr.ip().to_string())
                        .or(p.last_addr),
                    capabilities: info.map(|i| i.capabilities),
                    screens: info.map(|i| i.screens.clone()).unwrap_or_default(),
                    app_version: info.map(|i| i.app_version.clone()),
                    id: p.id,
                }
            })
            .collect();
        list.sort_by_key(|d| d.name.to_lowercase());
        let mut nearby: Vec<DeviceView> = st
            .nearby
            .values()
            .filter(|f| st.trust.get(&f.id).is_none())
            .map(|f| DeviceView {
                id: f.id.clone(),
                name: f.name.clone(),
                kind: f.kind,
                os: f.os,
                paired: false,
                online: false,
                nearby: true,
                address: f.addrs.first().map(|a| a.ip().to_string()),
                capabilities: None,
                screens: Vec::new(),
                app_version: None,
            })
            .collect();
        nearby.sort_by_key(|d| d.name.to_lowercase());
        list.extend(nearby);
        list
    }

    /// Shows a new pairing code on this device.
    pub fn begin_pairing(&self) -> PairingOffer {
        let code = pairing::generate_code();
        self.inner.st().pairing = Some(PairingWindow {
            code: code.clone(),
            expires: Instant::now() + Duration::from_secs(PAIRING_SECONDS),
            attempts_left: PAIRING_ATTEMPTS,
        });
        PairingOffer {
            code,
            seconds: PAIRING_SECONDS,
        }
    }

    pub fn cancel_pairing(&self) {
        self.inner.st().pairing = None;
    }

    /// Pairs with the device showing `code`. `target` is a device id from [`Node::devices`] or
    /// an address such as `192.168.1.20` or `192.168.1.20:47800`.
    pub async fn pair(&self, target: &str, code: &str) -> Result<DeviceId, NodeError> {
        let code = pairing::normalize_code(code).ok_or(NodeError::BadCode)?;
        let target = target.trim();
        let addrs: Vec<SocketAddr> = {
            let st = self.inner.st();
            match st.nearby.get(&DeviceId(target.to_string())) {
                Some(found) => found.addrs.clone(),
                None => parse_address(target).into_iter().collect(),
            }
        };
        if addrs.is_empty() {
            return Err(NodeError::NotFound);
        }
        let mut last = NodeError::Unreachable(target.to_string());
        for addr in addrs {
            let Ok(Ok(stream)) = tokio::time::timeout(DIAL_TIMEOUT, TcpStream::connect(addr)).await
            else {
                last = NodeError::Unreachable(addr.to_string());
                continue;
            };
            let (done, result) = oneshot::channel();
            let inner = self.inner.clone();
            // The session keeps running after pairing: it becomes the first trusted link.
            self.inner.rt.spawn(session::run(
                inner,
                stream,
                addr,
                true,
                Some(PairRequest {
                    code: code.clone(),
                    done,
                }),
            ));
            return result.await.unwrap_or(Err(NodeError::Interrupted));
        }
        Err(last)
    }

    /// Forgets a paired device, here and on the device itself.
    pub fn unpair(&self, id: &DeviceId) -> Result<(), NodeError> {
        {
            let mut st = self.inner.st();
            self.inner.send_to(&st, id, &Message::Unpaired);
            st.trust.remove(id)?;
            st.infos.remove(id);
            st.watching.retain(|(d, _), _| d != id);
            st.frames.retain(|(d, _), _| d != id);
            // Give the farewell a moment to leave before the link is dropped.
            if let Some(conn) = st.conns.remove(id) {
                self.inner.rt.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    drop(conn);
                });
            }
        }
        self.inner.emit(NodeEvent::Devices);
        Ok(())
    }

    /// Asks `device` for a live stream of one of its screens. Frames arrive as
    /// [`NodeEvent::Frame`]. The request is remembered and repeated when the device reconnects.
    pub fn watch(&self, device: &DeviceId, screen: u32, options: StreamOptions) {
        let mut st = self.inner.st();
        st.watching.insert((device.clone(), screen), options);
        self.inner
            .send_to(&st, device, &Message::StreamStart { screen, options });
    }

    pub fn unwatch(&self, device: &DeviceId, screen: u32) {
        let mut st = self.inner.st();
        st.watching.remove(&(device.clone(), screen));
        st.frames.remove(&(device.clone(), screen));
        self.inner
            .send_to(&st, device, &Message::StreamStop { screen });
    }

    /// Latest frame received for a watched screen.
    pub fn frame(&self, device: &DeviceId, screen: u32) -> Option<Arc<Frame>> {
        self.inner
            .st()
            .frames
            .get(&(device.clone(), screen))
            .cloned()
    }

    /// What the operating system lets this device share (only macOS asks).
    pub fn permissions(&self) -> Permissions {
        self.inner.platform.permissions()
    }

    /// Shows the system prompt and settings page for a permission.
    pub fn request_permission(&self, kind: PermissionKind) {
        self.inner.platform.request_permission(kind)
    }

    /// Sends keyboard or pointer input to a paired device.
    pub fn send_input(&self, device: &DeviceId, event: InputEvent) -> Result<(), NodeError> {
        let st = self.inner.st();
        if self.inner.send_to(&st, device, &Message::Input(event)) {
            Ok(())
        } else {
            Err(NodeError::Offline)
        }
    }
}

/// `192.168.1.20`, `192.168.1.20:47800`, `[fe80::1]:47800`.
fn parse_address(text: &str) -> Option<SocketAddr> {
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Some(addr);
    }
    text.parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, DEFAULT_PORT))
}

/// This computer's address on the local network, for pairing by address. No packet is sent:
/// the OS is only asked which interface it would use to reach the internet.
pub fn local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("192.168.1.20"),
            Some("192.168.1.20:47800".parse().unwrap())
        );
        assert_eq!(
            parse_address("192.168.1.20:5000"),
            Some("192.168.1.20:5000".parse().unwrap())
        );
        assert_eq!(parse_address("my-laptop"), None);
    }
}
