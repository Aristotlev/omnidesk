//! End-to-end tests: real nodes talking over real TCP on this machine, with generated screens.

use omnidesk_node::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct TestNode {
    node: Node,
    events: UnboundedReceiver<NodeEvent>,
    device: Arc<Synthetic>,
    dir: PathBuf,
}

impl TestNode {
    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.node.port())
    }
}

impl Drop for TestNode {
    fn drop(&mut self) {
        self.node.shutdown();
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "omnidesk-test-{}-{}-{}",
        std::process::id(),
        name,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

async fn start_in(name: &str, dir: PathBuf, port: u16) -> TestNode {
    let device = Arc::new(Synthetic::new(name));
    let config = NodeConfig {
        data_dir: dir.clone(),
        port,
        mdns: false,
        app_version: "test".into(),
        platform: device.clone(),
    };
    let (node, events) = Node::start(config).await.expect("node starts");
    TestNode {
        node,
        events,
        device,
        dir,
    }
}

async fn start(name: &str) -> TestNode {
    // Any free port: tests run in parallel.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    start_in(name, temp_dir(name), port).await
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..400 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for: {what}");
}

async fn pair(a: &TestNode, b: &TestNode) {
    let offer = b.node.begin_pairing();
    let id = a
        .node
        .pair(&b.addr(), &offer.code)
        .await
        .expect("pairing succeeds");
    assert_eq!(id, b.node.id());
    wait_for("both sides online", || online(a, b) && online(b, a)).await;
}

fn online(a: &TestNode, b: &TestNode) -> bool {
    a.node
        .devices()
        .iter()
        .any(|d| d.id == b.node.id() && d.paired && d.online && d.capabilities.is_some())
}

fn is_jpeg(data: &[u8]) -> bool {
    data.len() > 4 && data[..2] == [0xFF, 0xD8] && data[data.len() - 2..] == [0xFF, 0xD9]
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_then_watch_control_and_share_the_clipboard() {
    let (mut a, b) = (start("Laptop").await, start("Desktop").await);
    pair(&a, &b).await;

    // Each side knows the other by name, with its screen.
    let seen = a
        .node
        .devices()
        .into_iter()
        .find(|d| d.id == b.node.id())
        .unwrap();
    assert_eq!(seen.name, "Desktop");
    assert_eq!(seen.screens.len(), 1);
    assert_eq!(seen.capabilities.unwrap().view, ViewCap::Always);
    assert_eq!(
        b.node
            .devices()
            .into_iter()
            .find(|d| d.id == a.node.id())
            .unwrap()
            .name,
        "Laptop"
    );

    // Watch: frames arrive, scaled as asked.
    a.node.watch(&b.node.id(), 0, StreamOptions::THUMBNAIL);
    let mut seqs = Vec::new();
    while seqs.len() < 3 {
        if let NodeEvent::Frame {
            device,
            screen,
            seq,
            width,
            height,
        } = tokio::time::timeout(Duration::from_secs(5), a.events.recv())
            .await
            .expect("frames keep coming")
            .unwrap()
        {
            assert_eq!((device, screen, width, height), (b.node.id(), 0, 640, 360));
            seqs.push(seq);
        }
    }
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "frames arrive in order: {seqs:?}"
    );
    let frame = a.node.frame(&b.node.id(), 0).unwrap();
    assert!(is_jpeg(&frame.data));

    // Switching to full size replaces the stream.
    a.node.watch(&b.node.id(), 0, StreamOptions::FOCUS);
    wait_for("full-size frame", || {
        a.node
            .frame(&b.node.id(), 0)
            .is_some_and(|f| f.width == 1280)
    })
    .await;

    // Stop: no more captures after a moment.
    a.node.unwatch(&b.node.id(), 0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let captured = b.device.frames_captured();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        b.device.frames_captured(),
        captured,
        "capture stops when nobody watches"
    );
    assert!(a.node.frame(&b.node.id(), 0).is_none());

    // Control: input lands on the other device in order.
    let events = vec![
        InputEvent::PointerAbs {
            screen: 0,
            x: 0.5,
            y: 0.25,
        },
        InputEvent::Button {
            button: MouseButton::Left,
            down: true,
        },
        InputEvent::Button {
            button: MouseButton::Left,
            down: false,
        },
        InputEvent::Key {
            code: "KeyH".into(),
            key: "h".into(),
            down: true,
        },
        InputEvent::Key {
            code: "KeyH".into(),
            key: "h".into(),
            down: false,
        },
        InputEvent::Scroll { dx: 0, dy: 2 },
    ];
    for e in &events {
        a.node.send_input(&b.node.id(), e.clone()).unwrap();
    }
    wait_for("input played", || b.device.inputs().len() == events.len()).await;
    assert_eq!(b.device.inputs(), events);

    // Clipboard: copy on one, paste on the other, both directions, without echo loops.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    a.device.clipboard_set("copied on the laptop");
    wait_for("clipboard reaches the desktop", || {
        b.device.clipboard_get().as_deref() == Some("copied on the laptop")
    })
    .await;
    b.device.clipboard_set("copied on the desktop");
    wait_for("clipboard reaches the laptop", || {
        a.device.clipboard_get().as_deref() == Some("copied on the desktop")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(
        a.device.clipboard_get().as_deref(),
        Some("copied on the desktop")
    );
    assert_eq!(
        b.device.clipboard_get().as_deref(),
        Some("copied on the desktop")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_codes_fail_and_use_up_the_code() {
    let (a, mut b) = (start("A").await, start("B").await);

    // Nobody is showing a code.
    assert!(matches!(
        a.node.pair(&b.addr(), "123456").await,
        Err(NodeError::NotAccepting)
    ));
    // Not a code at all.
    assert!(matches!(
        a.node.pair(&b.addr(), "12ab").await,
        Err(NodeError::BadCode)
    ));

    let offer = b.node.begin_pairing();
    let wrong = if offer.code == "000000" {
        "000001"
    } else {
        "000000"
    };
    for _ in 0..3 {
        assert!(matches!(
            a.node.pair(&b.addr(), wrong).await,
            Err(NodeError::WrongCode)
        ));
    }
    // Three wrong tries: the code is gone, even the right one no longer works.
    assert!(matches!(
        a.node.pair(&b.addr(), &offer.code).await,
        Err(NodeError::NotAccepting)
    ));
    assert!(a.node.devices().is_empty() && b.node.devices().is_empty());
    let mut told = 0;
    while let Ok(event) = b.events.try_recv() {
        if matches!(event, NodeEvent::PairingEnded { .. }) {
            told += 1;
        }
    }
    assert_eq!(
        told, 3,
        "the device showing the code is told about each wrong try"
    );

    // A fresh code works, spaces and all.
    let offer = b.node.begin_pairing();
    let spaced = format!("{} {}", &offer.code[..3], &offer.code[3..]);
    a.node.pair(&b.addr(), &spaced).await.unwrap();
    wait_for("paired", || online(&a, &b) && online(&b, &a)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_is_not_paired_gets_nothing() {
    use omnidesk_link::{connect, Identity};
    let b = start("Desktop").await;
    let stranger = Identity::generate().unwrap();
    let stream = tokio::net::TcpStream::connect(b.addr()).await.unwrap();
    let (mut reader, mut writer, auth) = connect(stream, &stranger).await.unwrap();
    assert_eq!(auth.id, b.node.id());

    // It tries to type and to watch without pairing.
    writer
        .send(
            &Message::Input(InputEvent::Text {
                text: "rm -rf".into(),
            })
            .encode()
            .unwrap(),
        )
        .await
        .unwrap();
    let _ = writer
        .send(
            &Message::StreamStart {
                screen: 0,
                options: StreamOptions::FOCUS,
            }
            .encode()
            .unwrap(),
        )
        .await;

    // The only answer is "you are not paired", then the line goes dead.
    let reply = Message::decode(&reader.recv().await.unwrap()).unwrap();
    assert_eq!(reply, Message::Unpaired);
    assert!(reader.recv().await.is_err());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(b.device.inputs().is_empty());
    assert_eq!(b.device.frames_captured(), 0);
    assert!(b.node.devices().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn paired_devices_reconnect_after_a_restart() {
    let (a, b) = (start("Laptop").await, start("Desktop").await);
    pair(&a, &b).await;
    a.node.watch(&b.node.id(), 0, StreamOptions::THUMBNAIL);
    wait_for("first frame", || a.node.frame(&b.node.id(), 0).is_some()).await;

    // The desktop quits.
    let (dir, port, id) = (b.dir.clone(), b.node.port(), b.node.id());
    drop(b);
    wait_for("laptop sees it offline", || {
        a.node
            .devices()
            .iter()
            .any(|d| d.id == id && d.paired && !d.online)
    })
    .await;
    assert!(a.node.frame(&id, 0).is_none());

    // It starts again with the same identity: no new pairing, and the stream resumes by itself.
    let b = start_in("Desktop", dir, port).await;
    assert_eq!(b.node.id(), id);
    wait_for("back online", || online(&a, &b) && online(&b, &a)).await;
    wait_for("stream resumes", || a.node.frame(&id, 0).is_some()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unpairing_removes_trust_on_both_devices() {
    let (a, b) = (start("Laptop").await, start("Desktop").await);
    pair(&a, &b).await;
    a.node.unpair(&b.node.id()).unwrap();
    wait_for("both forget", || {
        a.node.devices().is_empty() && b.node.devices().is_empty()
    })
    .await;
    // Neither can control the other any more.
    assert!(matches!(
        a.node
            .send_input(&b.node.id(), InputEvent::Text { text: "x".into() }),
        Err(NodeError::Offline)
    ));
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        a.node.devices().is_empty() && b.node.devices().is_empty(),
        "they do not reconnect"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sharing_switches_are_enforced_by_the_shared_device() {
    let (mut a, b) = (start("Laptop").await, start("Desktop").await);
    pair(&a, &b).await;

    b.node.set_settings(Settings {
        allow_control: false,
        ..b.node.settings()
    });
    wait_for("laptop sees control off", || {
        a.node
            .devices()
            .iter()
            .any(|d| d.capabilities.is_some_and(|c| !c.control))
    })
    .await;
    a.node
        .send_input(
            &b.node.id(),
            InputEvent::Text {
                text: "ignored".into(),
            },
        )
        .unwrap();

    b.node.set_settings(Settings {
        allow_view: false,
        ..b.node.settings()
    });
    wait_for("laptop sees view off", || {
        a.node.devices().iter().any(|d| {
            d.capabilities.is_some_and(|c| c.view == ViewCap::None) && d.screens.is_empty()
        })
    })
    .await;
    a.node.watch(&b.node.id(), 0, StreamOptions::THUMBNAIL);
    let mut refused = false;
    for _ in 0..200 {
        match tokio::time::timeout(Duration::from_millis(50), a.events.recv()).await {
            Ok(Some(NodeEvent::StreamError { screen: 0, .. })) => {
                refused = true;
                break;
            }
            Ok(Some(NodeEvent::Frame { .. })) => panic!("a frame arrived although sharing is off"),
            _ => {}
        }
    }
    assert!(refused, "the laptop is told why there is no picture");
    assert!(b.device.inputs().is_empty());
    assert_eq!(b.device.frames_captured(), 0);

    // Renaming reaches the other device.
    b.node.set_settings(Settings {
        name: Some("Studio PC".into()),
        ..b.node.settings()
    });
    wait_for("new name", || {
        a.node.devices().iter().any(|d| d.name == "Studio PC")
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_controller_learns_why_control_does_nothing() {
    let (mut a, b) = (start("Laptop").await, start("MacBook").await);
    pair(&a, &b).await;
    b.device.deny_control(true);
    for _ in 0..3 {
        a.node
            .send_input(&b.node.id(), InputEvent::PointerRel { dx: 5, dy: 5 })
            .unwrap();
    }
    let mut notices = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(2), a.events.recv()).await
    {
        if let NodeEvent::Notice(text) = event {
            notices.push(text);
        }
    }
    assert_eq!(
        notices.len(),
        1,
        "told once, not once per event: {notices:?}"
    );
    assert!(
        notices[0].starts_with("MacBook: ") && notices[0].contains("Accessibility"),
        "{}",
        notices[0]
    );
    assert!(b.device.inputs().is_empty());
}
