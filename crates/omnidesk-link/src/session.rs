//! Noise XX handshake and the encrypted message channel that follows it.

use crate::{device_id, Identity, LinkError, KEY_LEN, NOISE_PARAMS};
use omnidesk_proto::DeviceId;
use snow::StatelessTransportState;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};

const PROLOGUE: &[u8] = b"omnidesk-link/1";
const MAX_RECORD: usize = 65535;
const TAG_LEN: usize = 16;
const MAX_PLAIN: usize = MAX_RECORD - TAG_LEN;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest message accepted from a paired device (a full-resolution frame fits many times over).
pub const MAX_MESSAGE: usize = 16 * 1024 * 1024;
/// Largest message accepted from a device that is not paired yet.
pub const UNPAIRED_MAX_MESSAGE: usize = 4096;

/// Who is on the other end, as proven by the handshake.
#[derive(Clone, Debug)]
pub struct PeerAuth {
    pub public_key: [u8; KEY_LEN],
    pub id: DeviceId,
    /// Unique to this connection. Pairing binds itself to it.
    pub handshake_hash: Vec<u8>,
    /// True when this side opened the connection.
    pub initiator: bool,
}

pub struct LinkReader<R> {
    io: R,
    cipher: Arc<StatelessTransportState>,
    nonce: u64,
    max_message: usize,
    record: Vec<u8>,
}

pub struct LinkWriter<W> {
    io: W,
    cipher: Arc<StatelessTransportState>,
    nonce: u64,
    record: Vec<u8>,
}

async fn write_frame<S: AsyncWrite + Unpin>(io: &mut S, bytes: &[u8]) -> Result<(), LinkError> {
    io.write_all(&(bytes.len() as u16).to_be_bytes()).await?;
    io.write_all(bytes).await?;
    io.flush().await?;
    Ok(())
}

async fn read_frame<S: AsyncRead + Unpin>(io: &mut S, buf: &mut Vec<u8>) -> Result<(), LinkError> {
    let mut len = [0u8; 2];
    io.read_exact(&mut len).await.map_err(eof)?;
    buf.resize(u16::from_be_bytes(len) as usize, 0);
    io.read_exact(buf).await.map_err(eof)?;
    Ok(())
}

fn eof(e: std::io::Error) -> LinkError {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::BrokenPipe => LinkError::Closed,
        _ => LinkError::Io(e),
    }
}

async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    identity: &Identity,
    initiator: bool,
) -> Result<(StatelessTransportState, PeerAuth), LinkError> {
    let builder = snow::Builder::new(NOISE_PARAMS.parse().expect("noise params"))
        .local_private_key(&identity.private_key)?
        .prologue(PROLOGUE)?;
    let mut state = if initiator {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    };
    let mut out = vec![0u8; MAX_RECORD];
    let mut inp = Vec::new();
    let mut payload = vec![0u8; MAX_RECORD];
    while !state.is_handshake_finished() {
        if state.is_my_turn() {
            let n = state.write_message(&[], &mut out)?;
            write_frame(stream, &out[..n]).await?;
        } else {
            read_frame(stream, &mut inp).await?;
            state.read_message(&inp, &mut payload)?;
        }
    }
    let remote = state
        .get_remote_static()
        .ok_or(LinkError::Protocol("no device key in handshake"))?;
    let public_key: [u8; KEY_LEN] = remote
        .try_into()
        .map_err(|_| LinkError::Protocol("device key has the wrong length"))?;
    let auth = PeerAuth {
        public_key,
        id: device_id(&public_key),
        handshake_hash: state.get_handshake_hash().to_vec(),
        initiator,
    };
    Ok((state.into_stateless_transport_mode()?, auth))
}

async fn open<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    identity: &Identity,
    initiator: bool,
) -> Result<(LinkReader<ReadHalf<S>>, LinkWriter<WriteHalf<S>>, PeerAuth), LinkError> {
    let (cipher, auth) = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        handshake(&mut stream, identity, initiator),
    )
    .await
    .map_err(|_| LinkError::Timeout)??;
    let cipher = Arc::new(cipher);
    let (r, w) = tokio::io::split(stream);
    Ok((
        LinkReader {
            io: r,
            cipher: cipher.clone(),
            nonce: 0,
            max_message: UNPAIRED_MAX_MESSAGE,
            record: Vec::new(),
        },
        LinkWriter {
            io: w,
            cipher,
            nonce: 0,
            record: vec![0u8; MAX_RECORD],
        },
        auth,
    ))
}

/// Opens a link as the side that dialled.
pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    identity: &Identity,
) -> Result<(LinkReader<ReadHalf<S>>, LinkWriter<WriteHalf<S>>, PeerAuth), LinkError> {
    open(stream, identity, true).await
}

/// Opens a link as the side that was dialled.
pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    identity: &Identity,
) -> Result<(LinkReader<ReadHalf<S>>, LinkWriter<WriteHalf<S>>, PeerAuth), LinkError> {
    open(stream, identity, false).await
}

impl<R: AsyncRead + Unpin> LinkReader<R> {
    /// Raises the size limit once the other device is known to be paired.
    pub fn set_max_message(&mut self, max: usize) {
        self.max_message = max.min(MAX_MESSAGE);
    }

    async fn read_record(&mut self, plain: &mut Vec<u8>) -> Result<(), LinkError> {
        read_frame(&mut self.io, &mut self.record).await?;
        let start = plain.len();
        plain.resize(start + self.record.len(), 0);
        let n = self
            .cipher
            .read_message(self.nonce, &self.record, &mut plain[start..])?;
        self.nonce += 1;
        plain.truncate(start + n);
        Ok(())
    }

    /// Next whole message. Fails with [`LinkError::Closed`] when the other side hangs up.
    pub async fn recv(&mut self) -> Result<Vec<u8>, LinkError> {
        let mut msg = Vec::new();
        self.read_record(&mut msg).await?;
        if msg.len() < 4 {
            return Err(LinkError::Protocol("short record"));
        }
        let total = u32::from_be_bytes([msg[0], msg[1], msg[2], msg[3]]) as usize;
        if total > self.max_message {
            return Err(LinkError::TooLarge(total));
        }
        msg.drain(..4);
        msg.reserve(total.saturating_sub(msg.len()));
        while msg.len() < total {
            self.read_record(&mut msg).await?;
        }
        if msg.len() != total {
            return Err(LinkError::Protocol("record crosses a message boundary"));
        }
        Ok(msg)
    }
}

impl<W: AsyncWrite + Unpin> LinkWriter<W> {
    /// Sends one whole message, split into encrypted records of at most 64 KiB.
    pub async fn send(&mut self, payload: &[u8]) -> Result<(), LinkError> {
        if payload.len() > MAX_MESSAGE {
            return Err(LinkError::TooLarge(payload.len()));
        }
        let header = (payload.len() as u32).to_be_bytes();
        let first_len = payload.len().min(MAX_PLAIN - 4);
        let mut first = Vec::with_capacity(4 + first_len);
        first.extend_from_slice(&header);
        first.extend_from_slice(&payload[..first_len]);
        self.write_record(&first).await?;
        for chunk in payload[first_len..].chunks(MAX_PLAIN) {
            self.write_record(chunk).await?;
        }
        self.io.flush().await.map_err(eof)
    }

    async fn write_record(&mut self, plain: &[u8]) -> Result<(), LinkError> {
        let n = self
            .cipher
            .write_message(self.nonce, plain, &mut self.record)?;
        self.nonce += 1;
        self.io
            .write_all(&(n as u16).to_be_bytes())
            .await
            .map_err(eof)?;
        self.io.write_all(&self.record[..n]).await.map_err(eof)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    async fn pair_of_links() -> (
        (
            LinkReader<ReadHalf<tokio::io::DuplexStream>>,
            LinkWriter<WriteHalf<tokio::io::DuplexStream>>,
            PeerAuth,
            Identity,
        ),
        (
            LinkReader<ReadHalf<tokio::io::DuplexStream>>,
            LinkWriter<WriteHalf<tokio::io::DuplexStream>>,
            PeerAuth,
            Identity,
        ),
    ) {
        let (a_io, b_io) = duplex(256 * 1024);
        let (a_id, b_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let (a, b) = tokio::join!(connect(a_io, &a_id), accept(b_io, &b_id));
        let (ar, aw, aa) = a.unwrap();
        let (br, bw, ba) = b.unwrap();
        ((ar, aw, aa, a_id), (br, bw, ba, b_id))
    }

    #[tokio::test]
    async fn handshake_proves_both_keys() {
        let ((_, _, a_auth, a_id), (_, _, b_auth, b_id)) = pair_of_links().await;
        assert_eq!(a_auth.public_key, b_id.public_key);
        assert_eq!(b_auth.public_key, a_id.public_key);
        assert_eq!(a_auth.id, b_id.id());
        assert_eq!(a_auth.handshake_hash, b_auth.handshake_hash);
        assert!(a_auth.initiator && !b_auth.initiator);
    }

    #[tokio::test]
    async fn messages_of_every_size_roundtrip_both_ways() {
        let ((mut ar, mut aw, ..), (mut br, mut bw, ..)) = pair_of_links().await;
        ar.set_max_message(MAX_MESSAGE);
        br.set_max_message(MAX_MESSAGE);
        for size in [
            0usize,
            1,
            4,
            MAX_PLAIN - 5,
            MAX_PLAIN - 4,
            MAX_PLAIN - 3,
            MAX_PLAIN,
            MAX_PLAIN + 1,
            200_000,
            1_500_000,
        ] {
            let msg: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let sent = msg.clone();
            let writer = tokio::spawn(async move {
                aw.send(&sent).await.unwrap();
                aw
            });
            assert_eq!(br.recv().await.unwrap(), msg, "size {size}");
            aw = writer.await.unwrap();
            bw.send(b"ack").await.unwrap();
            assert_eq!(ar.recv().await.unwrap(), b"ack");
        }
    }

    #[tokio::test]
    async fn unpaired_links_refuse_large_messages() {
        let ((_, mut aw, ..), (mut br, ..)) = pair_of_links().await;
        aw.send(&vec![0u8; UNPAIRED_MAX_MESSAGE + 1]).await.unwrap();
        assert!(matches!(br.recv().await, Err(LinkError::TooLarge(_))));
    }

    #[tokio::test]
    async fn tampered_bytes_are_rejected() {
        let (a_io, mut mid_a) = duplex(64 * 1024);
        let (mut mid_b, b_io) = duplex(64 * 1024);
        // Relay that flips one bit in the first record after the handshake.
        tokio::spawn(async move {
            let (mut ar, mut aw) = tokio::io::split(&mut mid_a);
            let (mut br, mut bw) = tokio::io::split(&mut mid_b);
            let forward = async {
                let mut frames = 0;
                loop {
                    let mut buf = Vec::new();
                    if read_frame(&mut ar, &mut buf).await.is_err() {
                        break;
                    }
                    frames += 1;
                    if frames == 3 {
                        let last = buf.len() - 1;
                        buf[last] ^= 1;
                    }
                    if write_frame(&mut bw, &buf).await.is_err() {
                        break;
                    }
                }
            };
            let backward = async {
                let _ = tokio::io::copy(&mut br, &mut aw).await;
            };
            tokio::join!(forward, backward);
        });
        let (a_id, b_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let (a, b) = tokio::join!(connect(a_io, &a_id), accept(b_io, &b_id));
        let (_, mut aw, _) = a.unwrap();
        let (mut br, _, _) = b.unwrap();
        aw.send(b"hello").await.unwrap();
        assert!(matches!(br.recv().await, Err(LinkError::Noise(_))));
    }

    #[tokio::test]
    async fn closing_one_side_ends_the_other() {
        let ((ar, aw, ..), (mut br, ..)) = pair_of_links().await;
        drop((ar, aw));
        assert!(matches!(br.recv().await, Err(LinkError::Closed)));
    }
}
