//! Finds other OmniDesk devices on the local network with mDNS (Bonjour).
//!
//! Each device announces `_omnidesk._tcp` with its id, name, type and port. Nothing secret is
//! announced: trust comes from pairing, never from discovery.

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use omnidesk_proto::{DeviceId, DeviceKind, Os, PROTOCOL_VERSION, SERVICE_TYPE};
use std::net::{IpAddr, SocketAddr};
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("local network discovery is unavailable: {0}")]
    Mdns(#[from] mdns_sd::Error),
}

/// What this device announces.
#[derive(Clone, Debug)]
pub struct Advert {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub os: Os,
    pub port: u16,
}

/// A device seen on the network.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub id: DeviceId,
    pub name: String,
    pub kind: DeviceKind,
    pub os: Os,
    /// Addresses to try, IPv4 first.
    pub addrs: Vec<SocketAddr>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Found(Found),
    Lost(DeviceId),
}

/// Running announcement and search. Dropping it stops both.
pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Discovery {
    pub fn start(
        advert: Advert,
    ) -> Result<(Discovery, mpsc::UnboundedReceiver<Event>), DiscoveryError> {
        let daemon = ServiceDaemon::new()?;
        let host = format!("omnidesk-{}.local.", advert.id);
        let version = PROTOCOL_VERSION.to_string();
        let props = [
            ("id", advert.id.as_str()),
            ("name", advert.name.as_str()),
            ("kind", advert.kind.as_str()),
            ("os", advert.os.as_str()),
            ("v", version.as_str()),
        ];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            advert.id.as_str(),
            &host,
            "",
            advert.port,
            &props[..],
        )?
        .enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;

        let browser = daemon.browse(SERVICE_TYPE)?;
        let (tx, rx) = mpsc::unbounded_channel();
        let own_id = advert.id.clone();
        std::thread::Builder::new()
            .name("omnidesk-mdns".into())
            .spawn(move || {
                while let Ok(event) = browser.recv() {
                    let out = match event {
                        ServiceEvent::ServiceResolved(svc) => {
                            parse(&svc, &own_id).map(Event::Found)
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => id_from_fullname(&fullname)
                            .filter(|id| *id != own_id)
                            .map(Event::Lost),
                        _ => None,
                    };
                    if let Some(out) = out {
                        if tx.send(out).is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("spawn mdns thread");
        Ok((Discovery { daemon, fullname }, rx))
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

fn id_from_fullname(fullname: &str) -> Option<DeviceId> {
    let id = fullname.split('.').next()?;
    (id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| DeviceId(id.to_string()))
}

fn parse(svc: &mdns_sd::ResolvedService, own_id: &DeviceId) -> Option<Found> {
    let id = DeviceId(svc.get_property_val_str("id")?.to_string());
    if id == *own_id || id_from_fullname(&svc.fullname).as_ref() != Some(&id) {
        return None;
    }
    let mut addrs: Vec<SocketAddr> = svc
        .addresses
        .iter()
        .map(|a| a.to_ip_addr())
        // Link-local IPv6 needs an interface scope we do not have; IPv4 or routable IPv6 always exists too.
        .filter(|ip| match ip {
            IpAddr::V4(v4) => !v4.is_unspecified(),
            IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) != 0xfe80 && !v6.is_unspecified(),
        })
        .map(|ip| SocketAddr::new(ip, svc.port))
        .collect();
    addrs.sort_by_key(|a| (a.is_ipv6(), a.ip().is_loopback(), a.to_string()));
    if addrs.is_empty() {
        return None;
    }
    Some(Found {
        name: svc
            .get_property_val_str("name")
            .unwrap_or("OmniDesk device")
            .to_string(),
        kind: svc
            .get_property_val_str("kind")
            .and_then(DeviceKind::parse)
            .unwrap_or(DeviceKind::Desktop),
        os: svc
            .get_property_val_str("os")
            .and_then(Os::parse)
            .unwrap_or(Os::Linux),
        id,
        addrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullname_to_id() {
        assert_eq!(
            id_from_fullname("ab12cd34ef567890._omnidesk._tcp.local."),
            Some(DeviceId("ab12cd34ef567890".into()))
        );
        assert_eq!(id_from_fullname("printer._omnidesk._tcp.local."), None);
    }
}
