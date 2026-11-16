//! This device's long-term key and the list of devices it trusts.

use crate::{LinkError, NOISE_PARAMS};
use omnidesk_proto::{DeviceId, DeviceKind, Os};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const KEY_LEN: usize = 32;

/// Device id derived from a public key.
pub fn device_id(public_key: &[u8]) -> DeviceId {
    let digest = Sha256::digest(public_key);
    DeviceId(hex::encode(&digest[..8]))
}

/// The X25519 key pair that identifies this device. Created once, then reused forever:
/// paired devices recognise each other by this key.
#[derive(Clone)]
pub struct Identity {
    pub private_key: [u8; KEY_LEN],
    pub public_key: [u8; KEY_LEN],
}

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    private_key: String,
    public_key: String,
}

impl Identity {
    pub fn generate() -> Result<Identity, LinkError> {
        let kp =
            snow::Builder::new(NOISE_PARAMS.parse().expect("noise params")).generate_keypair()?;
        let mut private_key = [0u8; KEY_LEN];
        let mut public_key = [0u8; KEY_LEN];
        private_key.copy_from_slice(&kp.private);
        public_key.copy_from_slice(&kp.public);
        Ok(Identity {
            private_key,
            public_key,
        })
    }

    pub fn id(&self) -> DeviceId {
        device_id(&self.public_key)
    }

    /// Loads `identity.json` from `dir`, creating it on first run.
    pub fn load_or_create(dir: &Path) -> Result<Identity, LinkError> {
        let path = dir.join("identity.json");
        if path.exists() {
            let file: IdentityFile = serde_json::from_slice(&fs::read(&path)?)?;
            let private_key = decode_key(&file.private_key)?;
            let public_key = decode_key(&file.public_key)?;
            return Ok(Identity {
                private_key,
                public_key,
            });
        }
        fs::create_dir_all(dir)?;
        let identity = Identity::generate()?;
        let file = IdentityFile {
            private_key: hex::encode(identity.private_key),
            public_key: hex::encode(identity.public_key),
        };
        write_private(&path, &serde_json::to_vec_pretty(&file)?)?;
        Ok(identity)
    }
}

fn decode_key(s: &str) -> Result<[u8; KEY_LEN], LinkError> {
    let bytes = hex::decode(s).map_err(|_| LinkError::Corrupt("key is not hex"))?;
    bytes
        .try_into()
        .map_err(|_| LinkError::Corrupt("key has the wrong length"))
}

/// Writes a file only its owner can read (0600 on Unix; the per-user profile folder on Windows).
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), LinkError> {
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp, bytes)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// A device this one has been paired with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub id: DeviceId,
    pub public_key: String,
    pub name: String,
    pub kind: DeviceKind,
    pub os: Os,
    /// Seconds since the Unix epoch.
    pub paired_at: u64,
    /// Last address the device was reached at, tried again when it is not announced on the network.
    #[serde(default)]
    pub last_addr: Option<String>,
}

/// Paired devices, saved as `peers.json`.
pub struct TrustStore {
    path: PathBuf,
    peers: BTreeMap<DeviceId, TrustedPeer>,
}

impl TrustStore {
    pub fn load(dir: &Path) -> Result<TrustStore, LinkError> {
        let path = dir.join("peers.json");
        let peers = if path.exists() {
            let list: Vec<TrustedPeer> = serde_json::from_slice(&fs::read(&path)?)?;
            list.into_iter().map(|p| (p.id.clone(), p)).collect()
        } else {
            BTreeMap::new()
        };
        Ok(TrustStore { path, peers })
    }

    pub fn in_memory() -> TrustStore {
        TrustStore {
            path: PathBuf::new(),
            peers: BTreeMap::new(),
        }
    }

    fn save(&self) -> Result<(), LinkError> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let list: Vec<&TrustedPeer> = self.peers.values().collect();
        write_private(&self.path, &serde_json::to_vec_pretty(&list)?)
    }

    pub fn is_trusted(&self, public_key: &[u8]) -> bool {
        self.peers
            .get(&device_id(public_key))
            .is_some_and(|p| p.public_key == hex::encode(public_key))
    }

    pub fn get(&self, id: &DeviceId) -> Option<&TrustedPeer> {
        self.peers.get(id)
    }

    pub fn list(&self) -> Vec<TrustedPeer> {
        self.peers.values().cloned().collect()
    }

    pub fn add(
        &mut self,
        public_key: &[u8],
        name: &str,
        kind: DeviceKind,
        os: Os,
    ) -> Result<TrustedPeer, LinkError> {
        let id = device_id(public_key);
        let paired_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last_addr = self.peers.get(&id).and_then(|p| p.last_addr.clone());
        let peer = TrustedPeer {
            id: id.clone(),
            public_key: hex::encode(public_key),
            name: name.to_string(),
            kind,
            os,
            paired_at,
            last_addr,
        };
        self.peers.insert(id, peer.clone());
        self.save()?;
        Ok(peer)
    }

    /// Updates the name, type and last address learned from a live connection.
    pub fn touch(
        &mut self,
        id: &DeviceId,
        name: Option<&str>,
        kind: Option<DeviceKind>,
        os: Option<Os>,
        addr: Option<String>,
    ) -> Result<(), LinkError> {
        let Some(peer) = self.peers.get_mut(id) else {
            return Ok(());
        };
        let before = peer.clone();
        if let Some(name) = name {
            peer.name = name.to_string();
        }
        if let Some(kind) = kind {
            peer.kind = kind;
        }
        if let Some(os) = os {
            peer.os = os;
        }
        if addr.is_some() {
            peer.last_addr = addr;
        }
        if *peer != before {
            self.save()?;
        }
        Ok(())
    }

    pub fn remove(&mut self, id: &DeviceId) -> Result<bool, LinkError> {
        let removed = self.peers.remove(id).is_some();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_survives_restart() {
        let dir = std::env::temp_dir().join(format!("omnidesk-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = Identity::load_or_create(&dir).unwrap();
        let b = Identity::load_or_create(&dir).unwrap();
        assert_eq!(a.public_key, b.public_key);
        assert_eq!(a.private_key, b.private_key);
        assert_eq!(a.id().0.len(), 16);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("identity.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trust_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("omnidesk-trust-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let peer = Identity::generate().unwrap();
        let stranger = Identity::generate().unwrap();
        {
            let mut store = TrustStore::load(&dir).unwrap();
            assert!(!store.is_trusted(&peer.public_key));
            store
                .add(&peer.public_key, "MacBook", DeviceKind::Laptop, Os::Macos)
                .unwrap();
            store
                .touch(
                    &peer.id(),
                    None,
                    None,
                    None,
                    Some("192.168.1.20:47800".into()),
                )
                .unwrap();
        }
        let mut store = TrustStore::load(&dir).unwrap();
        assert!(store.is_trusted(&peer.public_key));
        assert!(!store.is_trusted(&stranger.public_key));
        assert_eq!(
            store.get(&peer.id()).unwrap().last_addr.as_deref(),
            Some("192.168.1.20:47800")
        );
        assert!(store.remove(&peer.id()).unwrap());
        assert!(!TrustStore::load(&dir).unwrap().is_trusted(&peer.public_key));
        fs::remove_dir_all(&dir).unwrap();
    }
}
