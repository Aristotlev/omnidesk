//! One connection to another device: handshake, optional pairing, then the trusted link.

use crate::{text_hash, Inner, NodeError, NodeEvent};
use omnidesk_link::pairing::{self, Role};
use omnidesk_link::{accept, connect, LinkReader, LinkWriter, PeerAuth, MAX_MESSAGE};
use omnidesk_platform::{Platform, PlatformError};
use omnidesk_proto::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Notify};

type Reader = LinkReader<ReadHalf<TcpStream>>;
type Writer = LinkWriter<WriteHalf<TcpStream>>;

const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
const PING_EVERY: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_STREAMS_PER_DEVICE: usize = 8;
const REJECT_NOT_ACCEPTING: &str = "not_accepting";

pub(crate) struct PairRequest {
    pub code: String,
    pub done: oneshot::Sender<Result<DeviceId, NodeError>>,
}

/// Newest encoded frame per screen, waiting to be written. A slow link drops frames instead of
/// queueing them.
#[derive(Default)]
pub(crate) struct FrameSlots {
    slots: Mutex<HashMap<u32, Vec<u8>>>,
    notify: Notify,
}

impl FrameSlots {
    fn put(&self, screen: u32, message: Vec<u8>) {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(screen, message);
        self.notify.notify_one();
    }
    fn take_all(&self) -> Vec<Vec<u8>> {
        self.slots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, m)| m)
            .collect()
    }
}

/// A live trusted link, as kept in the node's state. Dropping it closes the connection, stops
/// the streams it was serving and releases any keys the other device was holding down.
pub(crate) struct Conn {
    pub serial: u64,
    pub out: mpsc::UnboundedSender<Vec<u8>>,
    pub addr: SocketAddr,
    pub initiated_by_us: bool,
    frames: Arc<FrameSlots>,
    streams: HashMap<u32, Arc<AtomicBool>>,
    held_keys: HashMap<String, String>,
    held_buttons: Vec<MouseButton>,
    /// The other device was already told why its input does nothing here.
    told_input_problem: bool,
    platform: Arc<dyn Platform>,
    _close: watch::Sender<bool>,
}

impl Conn {
    pub fn stop_streams(&mut self) {
        for (_, stop) in self.streams.drain() {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.stop_streams();
        for (code, key) in self.held_keys.drain() {
            let _ = self.platform.input(InputEvent::Key {
                code,
                key,
                down: false,
            });
        }
        for button in self.held_buttons.drain(..) {
            let _ = self.platform.input(InputEvent::Button {
                button,
                down: false,
            });
        }
    }
}

async fn send(writer: &mut Writer, message: &Message) -> Result<(), NodeError> {
    let bytes = message.encode().map_err(|_| NodeError::Interrupted)?;
    Ok(writer.send(&bytes).await?)
}

/// Sends a last message and closes so that it is not lost. Closing a connection while the
/// other side's data is still unread makes Windows reset it, which throws away what was just
/// sent. So: send, half-close, read and discard until the other side hangs up (briefly), close.
async fn send_and_close(mut reader: Reader, mut writer: Writer, message: &Message) {
    if send(&mut writer, message).await.is_err() {
        return;
    }
    writer.shutdown().await;
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while reader.recv().await.is_ok() {}
    })
    .await;
}

async fn recv(reader: &mut Reader, wait: Duration) -> Result<Message, NodeError> {
    let bytes = tokio::time::timeout(wait, reader.recv())
        .await
        .map_err(|_| NodeError::Interrupted)??;
    Message::decode(&bytes).map_err(|_| NodeError::Interrupted)
}

/// Adds the device on the other end to the paired list. Returns the name it is saved under.
fn trust_peer(inner: &Inner, peer: &PeerAuth) -> String {
    let mut st = inner.st();
    let (name, kind, os) = st
        .nearby
        .get(&peer.id)
        .map(|f| (f.name.clone(), f.kind, f.os))
        .unwrap_or(("New device".to_string(), DeviceKind::Desktop, Os::Linux));
    if let Err(e) = st.trust.add(&peer.public_key, &name, kind, os) {
        tracing::warn!("could not save the paired device: {e}");
    }
    name
}

/// Handshake, then either pairing or a check that the other device is already paired.
/// On the answering side the first message has to be read to know which; it is returned.
async fn establish(
    inner: &Arc<Inner>,
    stream: TcpStream,
    initiator: bool,
    pair_code: Option<&str>,
) -> Result<(Reader, Writer, PeerAuth, Option<Message>), NodeError> {
    let (mut reader, mut writer, peer) = if initiator {
        connect(stream, &inner.identity).await?
    } else {
        accept(stream, &inner.identity).await?
    };
    if peer.id == inner.id {
        return Err(NodeError::Interrupted);
    }
    let own_key = inner.identity.public_key;

    if initiator {
        let Some(code) = pair_code else {
            return if inner.st().trust.is_trusted(&peer.public_key) {
                Ok((reader, writer, peer, None))
            } else {
                Err(NodeError::Interrupted)
            };
        };
        let (attempt, spake) = pairing::start(
            Role::Initiator,
            code,
            &own_key,
            &peer.public_key,
            &peer.handshake_hash,
        );
        send(&mut writer, &Message::PairInit { spake }).await?;
        match recv(&mut reader, PAIR_TIMEOUT).await? {
            Message::PairReply { spake, confirm } => {
                let keys = attempt.finish(&spake)?;
                pairing::verify(&keys.responder, &confirm).map_err(|_| NodeError::WrongCode)?;
                send(
                    &mut writer,
                    &Message::PairConfirm {
                        confirm: keys.initiator,
                    },
                )
                .await?;
            }
            Message::PairReject { reason } if reason == REJECT_NOT_ACCEPTING => {
                return Err(NodeError::NotAccepting)
            }
            Message::PairReject { .. } => return Err(NodeError::WrongCode),
            _ => return Err(NodeError::Interrupted),
        }
        trust_peer(inner, &peer);
        return Ok((reader, writer, peer, None));
    }

    match recv(&mut reader, PAIR_TIMEOUT).await? {
        Message::PairInit { spake } => {
            let Some(code) = inner.pairing_code() else {
                let reject = Message::PairReject {
                    reason: REJECT_NOT_ACCEPTING.into(),
                };
                send_and_close(reader, writer, &reject).await;
                return Err(NodeError::NotAccepting);
            };
            let (attempt, reply) = pairing::start(
                Role::Responder,
                &code,
                &peer.public_key,
                &own_key,
                &peer.handshake_hash,
            );
            let Ok(keys) = attempt.finish(&spake) else {
                inner.pairing_result(false);
                return Err(NodeError::WrongCode);
            };
            send(
                &mut writer,
                &Message::PairReply {
                    spake: reply,
                    confirm: keys.responder,
                },
            )
            .await?;
            match recv(&mut reader, PAIR_TIMEOUT).await {
                Ok(Message::PairConfirm { confirm })
                    if pairing::verify(&keys.initiator, &confirm).is_ok() => {}
                _ => {
                    inner.pairing_result(false);
                    return Err(NodeError::WrongCode);
                }
            }
            inner.pairing_result(true);
            let name = trust_peer(inner, &peer);
            inner.emit(NodeEvent::Paired {
                device: peer.id.clone(),
                name,
            });
            Ok((reader, writer, peer, None))
        }
        first => {
            if inner.st().trust.is_trusted(&peer.public_key) {
                Ok((reader, writer, peer, Some(first)))
            } else {
                // A device that still lists this one as paired learns that it no longer is.
                send_and_close(reader, writer, &Message::Unpaired).await;
                Err(NodeError::Interrupted)
            }
        }
    }
}

/// Runs one connection from handshake to hang-up.
pub(crate) async fn run(
    inner: Arc<Inner>,
    stream: TcpStream,
    addr: SocketAddr,
    initiator: bool,
    pair: Option<PairRequest>,
) {
    let _ = stream.set_nodelay(true);
    let code = pair.as_ref().map(|p| p.code.clone());
    match establish(&inner, stream, initiator, code.as_deref()).await {
        Ok((reader, writer, peer, first)) => {
            if let Some(pair) = pair {
                let _ = pair.done.send(Ok(peer.id.clone()));
            }
            trusted(inner, reader, writer, peer, addr, initiator, first).await;
        }
        Err(e) => {
            tracing::debug!("connection with {addr} ended before it was trusted: {e}");
            if let Some(pair) = pair {
                let _ = pair.done.send(Err(e));
            }
        }
    }
}

async fn write_loop(
    mut writer: Writer,
    mut out: mpsc::UnboundedReceiver<Vec<u8>>,
    frames: Arc<FrameSlots>,
) {
    let mut ping = tokio::time::interval(PING_EVERY);
    let mut n = 0u64;
    loop {
        tokio::select! {
            biased;
            message = out.recv() => {
                let Some(message) = message else { break };
                if writer.send(&message).await.is_err() { break }
            }
            _ = frames.notify.notified() => {
                for frame in frames.take_all() {
                    if writer.send(&frame).await.is_err() { return }
                }
            }
            _ = ping.tick() => {
                n += 1;
                let Ok(bytes) = Message::Ping(n).encode() else { break };
                if writer.send(&bytes).await.is_err() { break }
            }
        }
    }
}

async fn trusted(
    inner: Arc<Inner>,
    mut reader: Reader,
    writer: Writer,
    peer: PeerAuth,
    addr: SocketAddr,
    initiator: bool,
    first: Option<Message>,
) {
    reader.set_max_message(MAX_MESSAGE);
    let serial = inner.serial.fetch_add(1, Ordering::Relaxed);
    let (out, out_rx) = mpsc::unbounded_channel();
    let (close_tx, mut close_rx) = watch::channel(false);
    let frames = Arc::new(FrameSlots::default());
    {
        let mut st = inner.st();
        if let Some(old) = st.conns.get(&peer.id) {
            // Both devices may dial each other at once. Keep the link dialled by the smaller id.
            let we_should_dial = inner.id < peer.id;
            let new_is_preferred = initiator == we_should_dial;
            let old_is_preferred = old.initiated_by_us == we_should_dial;
            if old_is_preferred && !new_is_preferred {
                return;
            }
        }
        let conn = Conn {
            serial,
            out,
            addr,
            initiated_by_us: initiator,
            frames: frames.clone(),
            streams: HashMap::new(),
            held_keys: HashMap::new(),
            held_buttons: Vec::new(),
            told_input_problem: false,
            platform: inner.platform.clone(),
            _close: close_tx,
        };
        st.conns.insert(peer.id.clone(), conn);
        let hello = Message::Hello(inner.info(&st));
        inner.send_to(&st, &peer.id, &hello);
        for ((device, screen), options) in st.watching.iter().filter(|((d, _), _)| *d == peer.id) {
            inner.send_to(
                &st,
                device,
                &Message::StreamStart {
                    screen: *screen,
                    options: *options,
                },
            );
        }
    }
    inner.emit(NodeEvent::Devices);
    let writer_task = tokio::spawn(write_loop(writer, out_rx, frames));

    let mut open = first.is_none_or(|m| handle(&inner, &peer.id, serial, addr, m));
    while open {
        tokio::select! {
            received = tokio::time::timeout(IDLE_TIMEOUT, reader.recv()) => {
                let Ok(Ok(bytes)) = received else { break };
                // Messages from a newer version that this build does not know are skipped.
                if let Ok(message) = Message::decode(&bytes) {
                    open = handle(&inner, &peer.id, serial, addr, message);
                }
            }
            _ = close_rx.changed() => break,
        }
    }

    writer_task.abort();
    let ended = {
        let mut st = inner.st();
        if st.conns.get(&peer.id).is_some_and(|c| c.serial == serial) {
            st.frames.retain(|(d, _), _| *d != peer.id);
            st.conns.remove(&peer.id)
        } else {
            None
        }
    };
    if ended.is_some() {
        drop(ended);
        inner.emit(NodeEvent::Devices);
    }
}

/// Acts on one message from a paired device. Returns `false` to hang up.
fn handle(
    inner: &Arc<Inner>,
    peer: &DeviceId,
    serial: u64,
    addr: SocketAddr,
    message: Message,
) -> bool {
    match message {
        Message::Hello(info) => {
            if info.id != *peer {
                return false;
            }
            let mut st = inner.st();
            let listen = SocketAddr::new(addr.ip(), info.listen_port).to_string();
            if let Err(e) = st.trust.touch(
                peer,
                Some(&info.name),
                Some(info.kind),
                Some(info.os),
                Some(listen),
            ) {
                tracing::warn!("could not save device details: {e}");
            }
            st.infos.insert(peer.clone(), info);
            drop(st);
            inner.emit(NodeEvent::Devices);
        }
        Message::Ping(n) => {
            inner.send_to(&inner.st(), peer, &Message::Pong(n));
        }
        Message::Pong(_) => {}
        Message::StreamStart { screen, options } => {
            let mut st = inner.st();
            let allowed = st.settings.allow_view && st.screens.iter().any(|s| s.index == screen);
            let sharing_off = !st.settings.allow_view;
            let Some(conn) = st.conns.get_mut(peer).filter(|c| c.serial == serial) else {
                return false;
            };
            if let Some(old) = conn.streams.remove(&screen) {
                old.store(true, Ordering::Relaxed);
            }
            if !allowed || conn.streams.len() >= MAX_STREAMS_PER_DEVICE {
                let reason = if sharing_off {
                    "Screen sharing is turned off on that device."
                } else {
                    "That screen is not available."
                };
                if let Ok(bytes) = (Message::StreamError {
                    screen,
                    reason: reason.into(),
                })
                .encode()
                {
                    let _ = conn.out.send(bytes);
                }
                return true;
            }
            let stop = Arc::new(AtomicBool::new(false));
            conn.streams.insert(screen, stop.clone());
            let (frames, out, inner) = (conn.frames.clone(), conn.out.clone(), inner.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("omnidesk-capture-{screen}"))
                .spawn(move || capture_loop(inner, screen, options.clamped(), frames, out, stop));
            if let Err(e) = spawned {
                tracing::warn!("could not start screen capture: {e}");
            }
        }
        Message::StreamStop { screen } => {
            let mut st = inner.st();
            if let Some(stop) = st
                .conns
                .get_mut(peer)
                .filter(|c| c.serial == serial)
                .and_then(|c| c.streams.remove(&screen))
            {
                stop.store(true, Ordering::Relaxed);
            }
        }
        Message::Frame(frame) => {
            let mut st = inner.st();
            let key = (peer.clone(), frame.screen);
            // Frames nobody asked for are dropped.
            if st.watching.contains_key(&key) {
                let event = NodeEvent::Frame {
                    device: peer.clone(),
                    screen: frame.screen,
                    seq: frame.seq,
                    width: frame.width,
                    height: frame.height,
                };
                st.frames.insert(key, Arc::new(frame));
                drop(st);
                inner.emit(event);
            }
        }
        Message::StreamError { screen, reason } => {
            inner.emit(NodeEvent::StreamError {
                device: peer.clone(),
                screen,
                reason,
            });
        }
        Message::Input(event) => {
            let mut st = inner.st();
            if !st.settings.allow_control {
                return true;
            }
            let Some(conn) = st.conns.get_mut(peer).filter(|c| c.serial == serial) else {
                return false;
            };
            match &event {
                InputEvent::Key {
                    code,
                    key,
                    down: true,
                } => {
                    conn.held_keys.insert(code.clone(), key.clone());
                }
                InputEvent::Key {
                    code, down: false, ..
                } => {
                    conn.held_keys.remove(code);
                }
                InputEvent::Button { button, down } => {
                    conn.held_buttons.retain(|b| b != button);
                    if *down {
                        conn.held_buttons.push(*button);
                    }
                }
                _ => {}
            }
            drop(st);
            if let Err(e) = inner.platform.input(event) {
                let text = e.to_string();
                inner.notice_once(&text);
                // Tell the controlling device once per connection, so it is not left guessing.
                let mut st = inner.st();
                if let Some(conn) = st
                    .conns
                    .get_mut(peer)
                    .filter(|c| c.serial == serial && !c.told_input_problem)
                {
                    conn.told_input_problem = true;
                    if let Ok(bytes) = (Message::Notice { text }).encode() {
                        let _ = conn.out.send(bytes);
                    }
                }
            }
        }
        Message::Clipboard { text } => {
            let mut st = inner.st();
            if st.settings.share_clipboard && text.len() <= MAX_CLIPBOARD_BYTES {
                st.clipboard_seen = Some(text_hash(&text));
                drop(st);
                let platform = inner.platform.clone();
                inner
                    .rt
                    .spawn_blocking(move || platform.clipboard_set(&text));
            }
        }
        Message::Notice { text } => {
            let name = inner
                .st()
                .infos
                .get(peer)
                .map(|i| i.name.clone())
                .unwrap_or_else(|| "The other device".into());
            inner.emit(NodeEvent::Notice(format!("{name}: {text}")));
        }
        Message::Unpaired => {
            let mut st = inner.st();
            if let Err(e) = st.trust.remove(peer) {
                tracing::warn!("could not update paired devices: {e}");
            }
            st.infos.remove(peer);
            st.watching.retain(|(d, _), _| d != peer);
            drop(st);
            return false;
        }
        Message::PairInit { .. }
        | Message::PairReply { .. }
        | Message::PairConfirm { .. }
        | Message::PairReject { .. } => {}
    }
    true
}

/// Captures one screen for one watching device until told to stop.
fn capture_loop(
    inner: Arc<Inner>,
    screen: u32,
    options: StreamOptions,
    frames: Arc<FrameSlots>,
    out: mpsc::UnboundedSender<Vec<u8>>,
    stop: Arc<AtomicBool>,
) {
    let interval = Duration::from_millis(1000 / options.fps.max(1) as u64);
    let mut seq = 0u64;
    let mut last_image: Option<Vec<u8>> = None;
    let mut last_sent = Instant::now();
    let mut failures = 0u32;
    while !stop.load(Ordering::Relaxed) && !out.is_closed() {
        let started = Instant::now();
        match inner.platform.capture(screen, options) {
            Ok(image) => {
                failures = 0;
                // An unchanged screen is re-sent only now and then, as a sign of life.
                let unchanged = last_image.as_deref() == Some(image.data.as_slice());
                if !unchanged || last_sent.elapsed() >= Duration::from_secs(2) {
                    seq += 1;
                    let frame = Frame {
                        screen,
                        seq,
                        width: image.width,
                        height: image.height,
                        format: FrameFormat::Jpeg,
                        data: image.data,
                    };
                    let message = Message::Frame(frame);
                    if let Ok(bytes) = message.encode() {
                        frames.put(screen, bytes);
                    }
                    if let Message::Frame(frame) = message {
                        last_image = Some(frame.data);
                    }
                    last_sent = Instant::now();
                }
            }
            Err(e) => {
                failures += 1;
                if failures == 1 {
                    tracing::warn!("screen {screen}: {e}");
                    let reason = match e {
                        PlatformError::Permission(_) => e.to_string(),
                        _ => "That device could not capture its screen.".to_string(),
                    };
                    if let Ok(bytes) = (Message::StreamError { screen, reason }).encode() {
                        let _ = out.send(bytes);
                    }
                    if !matches!(e, PlatformError::Permission(_)) {
                        inner.notice_once("This screen could not be captured for another device.");
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        std::thread::sleep(interval.saturating_sub(started.elapsed()));
    }
}
