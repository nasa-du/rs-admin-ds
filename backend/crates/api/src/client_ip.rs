//! Client address extraction for rate limiting and audit
//! (docs/specs/login-system.md §8.4).
//!
//! Only the `X-Forwarded-For` header is consulted, and only when the TCP peer
//! is inside `TRUSTED_PROXY_CIDRS`. The server must be started with
//! `into_make_service_with_connect_info::<SocketAddr>()`; a request without
//! peer information is rejected rather than sharing an IP bucket.

use crate::error::ApiError;
use crate::router::AppState;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use rs_admin_domain::ErrorCode;
use rs_admin_security::ClientIpSource;
use std::net::{IpAddr, SocketAddr};

/// The resolved client address, normalized to IPv4 when it was IPv4-mapped.
#[derive(Clone, Copy, Debug)]
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let services = state.auth().ok_or_else(ApiError::service_unavailable)?;
        let Some(ConnectInfo(peer)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() else {
            tracing::error!("request is missing TCP peer information");
            return Err(ApiError::new(ErrorCode::InternalError));
        };

        let forwarded = forwarded_for(&parts.headers);
        let resolution = services.resolve_client_ip(peer.ip(), forwarded.as_deref());
        if resolution.source == ClientIpSource::InsecureFallback {
            // ADR-0005 §9.4: the address itself belongs in the audit row, not
            // in application logs.
            tracing::warn!("unusable X-Forwarded-For; using the direct peer address");
        }

        Ok(Self(resolution.ip))
    }
}

/// Join every `X-Forwarded-For` header line with commas. A header value that is
/// not valid UTF-8 becomes an unparseable marker so the resolver falls back to
/// the peer instead of silently trusting part of the chain.
fn forwarded_for(headers: &axum::http::HeaderMap) -> Option<String> {
    let mut present = false;
    let mut joined = String::new();
    for value in headers.get_all("x-forwarded-for") {
        present = true;
        let Ok(text) = value.to_str() else {
            return Some("<invalid>".to_owned());
        };
        if !joined.is_empty() {
            joined.push(',');
        }
        joined.push_str(text);
    }
    present.then_some(joined)
}
