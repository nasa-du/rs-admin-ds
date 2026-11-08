//! Trusted-proxy client IP resolution and keyed throttle identifiers
//! (docs/specs/login-system.md §8.3–§8.4).
//!
//! Resolution never trusts a forwarding header unless the TCP peer is inside
//! the configured CIDR list. When the chain cannot be parsed, or every hop is
//! itself trusted, the direct peer is used and the caller is expected to log a
//! warning. Throttle keys are HMACs so the database never stores a plaintext
//! address.

use crate::HmacKey;
use hmac::{Hmac, Mac};
use ipnet::IpNet;
use sha2::Sha256;
use std::net::{IpAddr, Ipv6Addr};

/// Account-layer lock thresholds and durations (spec §8.3). Unlike the IP and
/// global thresholds these are fixed, not configurable.
pub const ACCOUNT_LOCK_THRESHOLD_1: i32 = 5;
pub const ACCOUNT_LOCK_THRESHOLD_2: i32 = 10;
pub const ACCOUNT_LOCK_THRESHOLD_3: i32 = 15;
pub const ACCOUNT_LOCK_SECONDS_1: i64 = 30;
pub const ACCOUNT_LOCK_SECONDS_2: i64 = 5 * 60;
pub const ACCOUNT_LOCK_SECONDS_3: i64 = 60 * 60;

/// Where the resolved client address came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientIpSource {
    /// The TCP peer address, because no forwarding header was usable.
    Peer,
    /// The first untrusted address in `X-Forwarded-For`.
    ForwardedFor,
    /// A trusted peer supplied a header that was malformed or contained only
    /// trusted hops; the request fell back to the peer address. Callers log
    /// this as a warning.
    InsecureFallback,
}

/// The client address selected for throttling, auditing and session records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIpResolution {
    pub ip: IpAddr,
    pub source: ClientIpSource,
}

/// A set of networks whose peers may supply `X-Forwarded-For`.
#[derive(Clone, Debug, Default)]
pub struct TrustedProxies {
    cidrs: Vec<IpNet>,
}

impl TrustedProxies {
    pub fn new(cidrs: Vec<IpNet>) -> Self {
        Self { cidrs }
    }

    /// Whether `ip` is a configured proxy. The address is normalized first so
    /// an IPv4-mapped peer matches an IPv4 network.
    fn is_trusted(&self, ip: IpAddr) -> bool {
        let ip = normalize_ip(ip);
        self.cidrs.iter().any(|network| network.contains(&ip))
    }

    /// Resolve the client address.
    ///
    /// `forwarded_for` must be the raw `X-Forwarded-For` value (multiple
    /// header lines joined with commas). It is only consulted when the peer is
    /// trusted; the chain is walked right to left, skipping trusted hops, and
    /// the first untrusted address wins.
    pub fn resolve(&self, peer: IpAddr, forwarded_for: Option<&str>) -> ClientIpResolution {
        let peer = normalize_ip(peer);
        let fallback = |source: ClientIpSource| ClientIpResolution { ip: peer, source };

        if !self.is_trusted(peer) {
            return fallback(ClientIpSource::Peer);
        }

        let Some(raw) = forwarded_for else {
            return fallback(ClientIpSource::Peer);
        };

        let mut chain = Vec::new();
        for entry in raw.split(',') {
            match entry.trim().parse::<IpAddr>() {
                Ok(ip) => chain.push(normalize_ip(ip)),
                Err(_) => return fallback(ClientIpSource::InsecureFallback),
            }
        }

        // Right to left: the nearest untrusted hop is the client as far as we
        // can prove; anything further left was supplied by an untrusted party.
        chain
            .iter()
            .rev()
            .find(|ip| !self.is_trusted(**ip))
            .map(|ip| ClientIpResolution {
                ip: *ip,
                source: ClientIpSource::ForwardedFor,
            })
            .unwrap_or_else(|| fallback(ClientIpSource::InsecureFallback))
    }
}

/// IPv4-mapped IPv6 addresses are treated as IPv4 everywhere (spec §8.4).
pub fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ipv6) => ipv6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        IpAddr::V4(_) => ip,
    }
}

/// Canonical value used in the IP throttle key: the IPv4 address, or the /64
/// network for IPv6 (spec §8.3).
pub fn throttle_address(ip: IpAddr) -> String {
    match normalize_ip(ip) {
        IpAddr::V4(ipv4) => ipv4.to_string(),
        IpAddr::V6(ipv6) => {
            let segments = ipv6.segments();
            let network = Ipv6Addr::new(
                segments[0],
                segments[1],
                segments[2],
                segments[3],
                0,
                0,
                0,
                0,
            );
            format!("{network}/64")
        }
    }
}

/// HMAC-SHA256 helper shared by the keyed throttle identifiers.
pub fn hmac_sha256(key: &HmacKey, input: &[u8]) -> [u8; 32] {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.expose()).expect("HMAC accepts keys of any length");
    mac.update(input);
    mac.finalize().into_bytes().into()
}

/// `HMAC-SHA256(THROTTLE_KEY, "ip:" || normalized address)`.
pub fn ip_throttle_key(key: &HmacKey, ip: IpAddr) -> [u8; 32] {
    hmac_sha256(key, format!("ip:{}", throttle_address(ip)).as_bytes())
}

/// `HMAC-SHA256(THROTTLE_KEY, "global")`.
pub fn global_throttle_key(key: &HmacKey) -> [u8; 32] {
    hmac_sha256(key, b"global")
}

/// `HMAC-SHA256(THROTTLE_KEY, normalized identifier)` for unknown identifiers.
pub fn identifier_throttle_key(key: &HmacKey, normalized_identifier: &str) -> [u8; 32] {
    hmac_sha256(key, normalized_identifier.as_bytes())
}

/// Lock duration applied to the account layer after `failed_count` consecutive
/// failures, or `None` below the first threshold.
pub fn account_lock_seconds(failed_count: i32) -> Option<i64> {
    if failed_count >= ACCOUNT_LOCK_THRESHOLD_3 {
        Some(ACCOUNT_LOCK_SECONDS_3)
    } else if failed_count >= ACCOUNT_LOCK_THRESHOLD_2 {
        Some(ACCOUNT_LOCK_SECONDS_2)
    } else if failed_count >= ACCOUNT_LOCK_THRESHOLD_1 {
        Some(ACCOUNT_LOCK_SECONDS_1)
    } else {
        None
    }
}
