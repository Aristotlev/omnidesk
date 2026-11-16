//! Pairing two devices with a short code.
//!
//! The device being added shows a 6-digit code; the other one types it. Both run SPAKE2 with
//! the code as password, inside an already encrypted but not yet trusted Noise link. The SPAKE2
//! identities are the two long-term keys seen in that handshake and the confirmation values are
//! bound to the handshake hash, so:
//!
//! * someone who does not know the code gets exactly one guess per code (1 in 1,000,000),
//!   because a code is thrown away after any attempt;
//! * a device relaying the connection ends up with different keys on each side and fails.

use crate::LinkError;
use hkdf::Hkdf;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};
use subtle::ConstantTimeEq;

pub const CODE_DIGITS: usize = 6;

/// A fresh random 6-digit code.
pub fn generate_code() -> String {
    // Rejection sampling keeps every code equally likely.
    loop {
        let mut buf = [0u8; 4];
        getrandom::fill(&mut buf).expect("system random number generator");
        let n = u32::from_le_bytes(buf);
        if n < 4_294_000_000 {
            return format!("{:06}", n % 1_000_000);
        }
    }
}

/// Accepts what people type ("123 456", "123-456") and returns the 6 digits, or `None`.
pub fn normalize_code(input: &str) -> Option<String> {
    let digits: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect();
    (digits.len() == CODE_DIGITS && digits.bytes().all(|b| b.is_ascii_digit())).then_some(digits)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The device that typed the code (and opened the connection).
    Initiator,
    /// The device that showed the code.
    Responder,
}

pub struct Attempt {
    spake: Spake2<Ed25519Group>,
    handshake_hash: Vec<u8>,
}

/// Confirmation values each side sends to prove it derived the same key.
pub struct Confirmations {
    pub initiator: [u8; 32],
    pub responder: [u8; 32],
}

fn spake_identity(role: &str, key: &[u8]) -> SpakeIdentity {
    SpakeIdentity::new(format!("omnidesk-pair-v1/{role}/{}", hex::encode(key)).as_bytes())
}

/// Starts a pairing attempt. Returns the state to keep and the SPAKE2 message to send.
///
/// `initiator_key` and `responder_key` are the long-term public keys from the Noise handshake,
/// `handshake_hash` is that handshake's hash.
pub fn start(
    role: Role,
    code: &str,
    initiator_key: &[u8],
    responder_key: &[u8],
    handshake_hash: &[u8],
) -> (Attempt, Vec<u8>) {
    let password = Password::new(code.as_bytes());
    let id_a = spake_identity("initiator", initiator_key);
    let id_b = spake_identity("responder", responder_key);
    let (spake, message) = match role {
        Role::Initiator => Spake2::<Ed25519Group>::start_a(&password, &id_a, &id_b),
        Role::Responder => Spake2::<Ed25519Group>::start_b(&password, &id_a, &id_b),
    };
    (
        Attempt {
            spake,
            handshake_hash: handshake_hash.to_vec(),
        },
        message,
    )
}

impl Attempt {
    /// Combines the other side's SPAKE2 message into the confirmation values.
    pub fn finish(self, peer_message: &[u8]) -> Result<Confirmations, LinkError> {
        let key = self
            .spake
            .finish(peer_message)
            .map_err(|_| LinkError::Protocol("bad pairing message"))?;
        let hk = Hkdf::<Sha256>::new(Some(&self.handshake_hash), &key);
        let mut initiator = [0u8; 32];
        let mut responder = [0u8; 32];
        hk.expand(b"omnidesk pair confirm initiator", &mut initiator)
            .expect("32 bytes is a valid length");
        hk.expand(b"omnidesk pair confirm responder", &mut responder)
            .expect("32 bytes is a valid length");
        Ok(Confirmations {
            initiator,
            responder,
        })
    }
}

/// Constant-time comparison of a received confirmation value.
pub fn verify(expected: &[u8; 32], received: &[u8; 32]) -> Result<(), LinkError> {
    if bool::from(expected.ct_eq(received)) {
        Ok(())
    } else {
        Err(LinkError::WrongCode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        code_a: &str,
        code_b: &str,
        hash_a: &[u8],
        hash_b: &[u8],
    ) -> (Confirmations, Confirmations) {
        let (ik, rk) = ([1u8; 32], [2u8; 32]);
        let (a, msg_a) = start(Role::Initiator, code_a, &ik, &rk, hash_a);
        let (b, msg_b) = start(Role::Responder, code_b, &ik, &rk, hash_b);
        (a.finish(&msg_b).unwrap(), b.finish(&msg_a).unwrap())
    }

    #[test]
    fn same_code_agrees() {
        let (a, b) = run("123456", "123456", b"hash", b"hash");
        assert!(verify(&a.responder, &b.responder).is_ok());
        assert!(verify(&b.initiator, &a.initiator).is_ok());
        assert_ne!(a.initiator, a.responder);
    }

    #[test]
    fn wrong_code_fails() {
        let (a, b) = run("123456", "123457", b"hash", b"hash");
        assert!(matches!(
            verify(&a.responder, &b.responder),
            Err(LinkError::WrongCode)
        ));
        assert!(verify(&b.initiator, &a.initiator).is_err());
    }

    #[test]
    fn relayed_handshake_fails_even_with_the_right_code() {
        // A device in the middle gives each side a different Noise handshake.
        let (a, b) = run("123456", "123456", b"hash seen by A", b"hash seen by B");
        assert!(verify(&a.responder, &b.responder).is_err());
    }

    #[test]
    fn different_keys_fail_even_with_the_right_code() {
        let (a, msg_a) = start(Role::Initiator, "123456", &[1; 32], &[2; 32], b"h");
        let (b, msg_b) = start(Role::Responder, "123456", &[9; 32], &[2; 32], b"h");
        let (ca, cb) = (a.finish(&msg_b).unwrap(), b.finish(&msg_a).unwrap());
        assert!(verify(&ca.responder, &cb.responder).is_err());
    }

    #[test]
    fn garbage_message_is_rejected() {
        let (a, _) = start(Role::Initiator, "123456", &[1; 32], &[2; 32], b"h");
        assert!(a.finish(b"nope").is_err());
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..200 {
            let code = generate_code();
            assert_eq!(normalize_code(&code).as_deref(), Some(code.as_str()));
        }
        assert_eq!(normalize_code(" 123 456 ").as_deref(), Some("123456"));
        assert_eq!(normalize_code("123-456").as_deref(), Some("123456"));
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("12345a"), None);
    }
}
