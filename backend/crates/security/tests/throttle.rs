//! Trusted-proxy client IP resolution and throttle key derivation
//! (docs/specs/login-system.md §8.3–§8.4).

use ipnet::IpNet;
use rs_admin_security::HmacKey;
use rs_admin_security::throttle::{
    ClientIpSource, TrustedProxies, account_lock_seconds, global_throttle_key, hmac_sha256,
    ip_throttle_key, normalize_ip, throttle_address,
};
use std::net::{IpAddr, Ipv6Addr};

fn ip(value: &str) -> IpAddr {
    value.parse().expect("ip literal")
}

fn proxies(cidrs: &[&str]) -> TrustedProxies {
    TrustedProxies::new(
        cidrs
            .iter()
            .map(|cidr| cidr.parse::<IpNet>().expect("valid cidr"))
            .collect(),
    )
}

fn key() -> HmacKey {
    HmacKey::from_bytes(vec![7u8; 32]).expect("key")
}

#[test]
fn an_untrusted_peer_never_uses_forwarded_headers() {
    let trusted = proxies(&["10.0.0.0/8"]);
    let resolution = trusted.resolve(ip("203.0.113.9"), Some("198.51.100.7, 10.0.0.2"));

    assert_eq!(resolution.ip, ip("203.0.113.9"));
    assert_eq!(resolution.source, ClientIpSource::Peer);
}

#[test]
fn an_empty_trust_list_ignores_forwarded_headers() {
    let trusted = TrustedProxies::default();
    let resolution = trusted.resolve(ip("203.0.113.9"), Some("198.51.100.7"));

    assert_eq!(resolution.ip, ip("203.0.113.9"));
    assert_eq!(resolution.source, ClientIpSource::Peer);
}

#[test]
fn a_trusted_peer_takes_the_first_untrusted_address_from_the_right() {
    let trusted = proxies(&["10.0.0.0/8", "192.168.0.0/16"]);
    // client, proxy-a, proxy-b; the direct peer is proxy-b.
    let resolution = trusted.resolve(ip("10.0.0.3"), Some("198.51.100.7, 192.168.0.4, 10.0.0.2"));

    assert_eq!(resolution.ip, ip("198.51.100.7"));
    assert_eq!(resolution.source, ClientIpSource::ForwardedFor);
}

#[test]
fn a_forwarded_chain_of_only_trusted_proxies_falls_back_to_the_peer() {
    let trusted = proxies(&["10.0.0.0/8"]);
    let resolution = trusted.resolve(ip("10.0.0.3"), Some("10.0.0.2, 10.0.0.1"));

    assert_eq!(resolution.ip, ip("10.0.0.3"));
    assert_eq!(resolution.source, ClientIpSource::InsecureFallback);
}

#[test]
fn an_invalid_forwarded_chain_falls_back_to_the_peer() {
    let trusted = proxies(&["10.0.0.0/8"]);

    for value in [
        "not-an-ip",
        "198.51.100.7, nope",
        "198.51.100.7,,10.0.0.2",
        "",
    ] {
        let resolution = trusted.resolve(ip("10.0.0.3"), Some(value));
        assert_eq!(resolution.ip, ip("10.0.0.3"), "value: {value:?}");
        assert_eq!(
            resolution.source,
            ClientIpSource::InsecureFallback,
            "value: {value:?}"
        );
    }
}

#[test]
fn a_missing_forwarded_header_uses_the_trusted_peer() {
    let trusted = proxies(&["10.0.0.0/8"]);
    let resolution = trusted.resolve(ip("10.0.0.3"), None);

    assert_eq!(resolution.ip, ip("10.0.0.3"));
    assert_eq!(resolution.source, ClientIpSource::Peer);
}

#[test]
fn ipv4_mapped_addresses_are_normalized_to_ipv4() {
    assert_eq!(normalize_ip(ip("::ffff:192.0.2.5")), ip("192.0.2.5"));

    let trusted = proxies(&["127.0.0.0/8"]);
    let resolution = trusted.resolve(ip("::ffff:127.0.0.1"), Some("::ffff:198.51.100.7"));

    assert_eq!(resolution.ip, ip("198.51.100.7"));
    assert_eq!(resolution.source, ClientIpSource::ForwardedFor);
}

#[test]
fn ipv6_throttle_addresses_are_aggregated_per_slash_64() {
    let in_network: Ipv6Addr = "2001:db8:1:2:aaaa::1".parse().expect("ipv6");
    let same_network: Ipv6Addr = "2001:db8:1:2:bbbb::9".parse().expect("ipv6");
    let next_network: Ipv6Addr = "2001:db8:1:3::1".parse().expect("ipv6");

    assert_eq!(
        throttle_address(IpAddr::V6(in_network)),
        "2001:db8:1:2::/64"
    );
    assert_eq!(
        ip_throttle_key(&key(), IpAddr::V6(in_network)),
        ip_throttle_key(&key(), IpAddr::V6(same_network))
    );
    assert_ne!(
        ip_throttle_key(&key(), IpAddr::V6(in_network)),
        ip_throttle_key(&key(), IpAddr::V6(next_network))
    );
}

#[test]
fn throttle_keys_are_domain_separated() {
    let key = key();
    let ip_key = ip_throttle_key(&key, ip("198.51.100.7"));
    let other_ip_key = ip_throttle_key(&key, ip("198.51.100.8"));
    let global_key = global_throttle_key(&key);

    assert_ne!(ip_key, other_ip_key);
    assert_ne!(ip_key, global_key);
    assert_ne!(global_key, hmac_sha256(&key, b"global2"));
}

#[test]
fn account_lock_thresholds_match_the_spec() {
    assert_eq!(account_lock_seconds(0), None);
    assert_eq!(account_lock_seconds(4), None);
    assert_eq!(account_lock_seconds(5), Some(30));
    assert_eq!(account_lock_seconds(9), Some(30));
    assert_eq!(account_lock_seconds(10), Some(300));
    assert_eq!(account_lock_seconds(14), Some(300));
    assert_eq!(account_lock_seconds(15), Some(3_600));
    assert_eq!(account_lock_seconds(99), Some(3_600));
}
