//! Shared list-query parsing (docs/specs/login-system.md §7.4).
//!
//! Query parameters arrive as strings through the generated client; these
//! helpers enforce the common pagination contract and the field-validation
//! problem shape.

use crate::error::{ApiError, FieldError};
use rs_admin_domain::ErrorCode;
use uuid::Uuid;

/// Default page size for every offset-paginated endpoint.
pub(crate) const DEFAULT_PAGE_SIZE: i64 = 20;

/// Hard upper bound; larger requests are rejected, never truncated.
pub(crate) const MAX_PAGE_SIZE: i64 = 200;

pub(crate) fn validation_error(pointer: &str, code: &str) -> ApiError {
    ApiError::new(ErrorCode::ValidationFailed).with_errors(vec![FieldError {
        pointer: pointer.to_owned(),
        code: code.to_owned(),
        params: None,
    }])
}

/// Parse `page` (one-based, default 1) and `page_size` (default 20, at most
/// 200). Rejects values that cannot be represented as an offset.
pub(crate) fn parse_pagination(
    page: Option<&str>,
    page_size: Option<&str>,
) -> Result<(i64, i64), ApiError> {
    let page = match page.map(str::trim) {
        None | Some("") => 1,
        Some(raw) => raw
            .parse::<i64>()
            .ok()
            .filter(|page| *page >= 1)
            .ok_or_else(|| validation_error("/page", "invalid"))?,
    };
    let page_size = match page_size.map(str::trim) {
        None | Some("") => DEFAULT_PAGE_SIZE,
        Some(raw) => {
            let size = raw
                .parse::<i64>()
                .ok()
                .filter(|size| *size >= 1)
                .ok_or_else(|| validation_error("/page_size", "invalid"))?;
            if size > MAX_PAGE_SIZE {
                return Err(ApiError::new(ErrorCode::PageSizeTooLarge));
            }
            size
        }
    };
    page.checked_sub(1)
        .and_then(|page| page.checked_mul(page_size))
        .ok_or_else(|| validation_error("/page", "invalid"))?;
    Ok((page, page_size))
}

/// Parse a path id; malformed ids are indistinguishable from missing
/// resources.
pub(crate) fn parse_id(raw: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(raw).map_err(|_| ApiError::new(ErrorCode::NotFound))
}
