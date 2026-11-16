//! Identity, pairing and the encrypted link between two OmniDesk devices.
//!
//! * [`Identity`] is the device's long-term X25519 key. [`TrustStore`] lists the keys it trusts.
//! * [`connect`] / [`accept`] run a Noise XX handshake over any byte stream (TCP on the local
//!   network today, a relay later) and return an encrypted, authenticated message channel.
//!   Both sides learn each other's long-term key; whether that key is trusted is up to the caller.
//! * [`pairing`] turns a 6-digit code into mutual trust (SPAKE2 bound to the Noise handshake),
//!   so a device in the middle cannot pair itself by guessing or relaying.

mod identity;
pub mod pairing;
mod session;

pub use identity::{device_id, Identity, TrustStore, TrustedPeer, KEY_LEN};
pub use session::{
    accept, connect, LinkReader, LinkWriter, PeerAuth, MAX_MESSAGE, UNPAIRED_MAX_MESSAGE,
};

/// Noise pattern used for every link.
pub const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("connection closed")]
    Closed,
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error("encryption error: {0}")]
    Noise(#[from] snow::Error),
    #[error("stored data is damaged: {0}")]
    Corrupt(&'static str),
    #[error("stored data is damaged: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the other device sent something unexpected: {0}")]
    Protocol(&'static str),
    #[error("message of {0} bytes is over the limit")]
    TooLarge(usize),
    #[error("the other device did not answer in time")]
    Timeout,
    #[error("the pairing code does not match")]
    WrongCode,
}
