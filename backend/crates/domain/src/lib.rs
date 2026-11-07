//! Pure domain types. This crate must not depend on HTTP, SQLx or Axum.

use std::fmt;
use std::str::FromStr;

/// The closed API error-code catalog (docs/specs/login-system.md §7.3).
///
/// New codes must be added here, to OpenAPI, to the frontend mapping and to
/// the tests in this module in the same change.
macro_rules! error_codes {
    ($( $variant:ident => ($name:literal, $status:literal, $title:literal, $detail:literal) ),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ErrorCode {
            $( $variant ),+
        }

        impl ErrorCode {
            /// Every code in the catalog, in the order defined by the spec.
            pub const ALL: [Self; 41] = [ $( Self::$variant ),+ ];

            /// Stable wire identifier.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $name ),+
                }
            }

            /// HTTP status paired with this code.
            pub const fn status(self) -> u16 {
                match self {
                    $( Self::$variant => $status ),+
                }
            }

            /// English fallback title (clients localize by `code`).
            pub const fn title(self) -> &'static str {
                match self {
                    $( Self::$variant => $title ),+
                }
            }

            /// English fallback detail. Handlers may replace it, but must not
            /// expose SQL, stack traces or other internal details.
            pub const fn detail(self) -> &'static str {
                match self {
                    $( Self::$variant => $detail ),+
                }
            }

            /// RFC 7807 `type` member.
            pub fn type_uri(self) -> String {
                format!("urn:rs-admin:problem:{}", self.as_str())
            }
        }

        impl FromStr for ErrorCode {
            type Err = UnknownErrorCode;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $( $name => Ok(Self::$variant), )+
                    _ => Err(UnknownErrorCode(value.to_owned())),
                }
            }
        }
    };
}

error_codes! {
    InvalidJson => (
        "invalid_json",
        400,
        "Invalid JSON",
        "The request body is not valid JSON."
    ),
    UnknownField => (
        "unknown_field",
        400,
        "Unknown field",
        "The request body contains unknown fields."
    ),
    UnsupportedMediaType => (
        "unsupported_media_type",
        415,
        "Unsupported media type",
        "The request content type is not supported."
    ),
    MethodNotAllowed => (
        "method_not_allowed",
        405,
        "Method not allowed",
        "The HTTP method is not allowed for this resource."
    ),
    PayloadTooLarge => (
        "payload_too_large",
        413,
        "Payload too large",
        "The request body exceeds the allowed size."
    ),
    ValidationFailed => (
        "validation_failed",
        422,
        "Validation failed",
        "One or more fields are invalid."
    ),
    InvalidSort => (
        "invalid_sort",
        422,
        "Invalid sort field",
        "The requested sort field is not allowed."
    ),
    PageSizeTooLarge => (
        "page_size_too_large",
        422,
        "Page size too large",
        "The page size exceeds the maximum of 200."
    ),
    EmptyUpdate => (
        "empty_update",
        422,
        "Empty update",
        "The update contains no fields."
    ),
    InvalidTimeRange => (
        "invalid_time_range",
        422,
        "Invalid time range",
        "The time range is invalid."
    ),
    TimeRangeTooLarge => (
        "time_range_too_large",
        422,
        "Time range too large",
        "The time range exceeds the maximum of 90 days."
    ),
    InvalidCredentials => (
        "invalid_credentials",
        401,
        "Invalid credentials",
        "The credentials are invalid."
    ),
    AccessTokenInvalid => (
        "access_token_invalid",
        401,
        "Access token invalid",
        "The access token is invalid or expired."
    ),
    SessionInvalid => (
        "session_invalid",
        401,
        "Session invalid",
        "The session is invalid or expired."
    ),
    RefreshTokenInvalid => (
        "refresh_token_invalid",
        401,
        "Refresh token invalid",
        "The refresh token is invalid or expired."
    ),
    RefreshReuseDetected => (
        "refresh_reuse_detected",
        401,
        "Refresh token reuse detected",
        "The refresh token was reused."
    ),
    AccountDisabled => (
        "account_disabled",
        403,
        "Account disabled",
        "The account is disabled."
    ),
    TemporaryPasswordExpired => (
        "temporary_password_expired",
        403,
        "Temporary password expired",
        "The temporary password has expired."
    ),
    LoginLocked => (
        "login_locked",
        429,
        "Login locked",
        "The account is temporarily locked."
    ),
    RateLimited => (
        "rate_limited",
        429,
        "Rate limited",
        "Too many requests."
    ),
    PasswordChangeRequired => (
        "password_change_required",
        403,
        "Password change required",
        "A password change is required."
    ),
    CsrfFailed => (
        "csrf_failed",
        403,
        "CSRF validation failed",
        "The request failed CSRF validation."
    ),
    ReauthRequired => (
        "reauth_required",
        403,
        "Reauthentication required",
        "Reauthentication is required."
    ),
    InvalidCurrentPassword => (
        "invalid_current_password",
        422,
        "Current password invalid",
        "The current password is invalid."
    ),
    Forbidden => (
        "forbidden",
        403,
        "Forbidden",
        "The request is not allowed."
    ),
    OutOfScope => (
        "out_of_scope",
        403,
        "Out of scope",
        "The target is outside the management scope."
    ),
    SelfActionForbidden => (
        "self_action_forbidden",
        403,
        "Self action forbidden",
        "The action cannot be performed on yourself."
    ),
    RoleImmutable => (
        "role_immutable",
        403,
        "Role immutable",
        "Built-in roles cannot be modified."
    ),
    PermissionSubsetViolation => (
        "permission_subset_violation",
        403,
        "Permission subset violation",
        "The permissions exceed your own permissions."
    ),
    NotFound => (
        "not_found",
        404,
        "Not found",
        "The requested resource was not found."
    ),
    IdentifierTaken => (
        "identifier_taken",
        409,
        "Identifier taken",
        "The identifier is already in use."
    ),
    RoleNameTaken => (
        "role_name_taken",
        409,
        "Role name taken",
        "The role name is already in use."
    ),
    RoleInUse => (
        "role_in_use",
        409,
        "Role in use",
        "The role is still assigned to users."
    ),
    LastSuperAdmin => (
        "last_super_admin",
        409,
        "Last super administrator",
        "The last super administrator cannot be removed."
    ),
    PasswordPolicyViolation => (
        "password_policy_violation",
        422,
        "Password policy violation",
        "The password does not satisfy the password policy."
    ),
    PasswordUnchanged => (
        "password_unchanged",
        422,
        "Password unchanged",
        "The new password matches the current password."
    ),
    UnknownRole => (
        "unknown_role",
        422,
        "Unknown role",
        "The role does not exist."
    ),
    EmptyPermissions => (
        "empty_permissions",
        422,
        "Empty permissions",
        "The role must have at least one permission."
    ),
    InvalidPermission => (
        "invalid_permission",
        422,
        "Invalid permission",
        "The permission is not in the catalog."
    ),
    InternalError => (
        "internal_error",
        500,
        "Internal error",
        "An internal error occurred."
    ),
    ServiceUnavailable => (
        "service_unavailable",
        503,
        "Service unavailable",
        "The service is temporarily unavailable."
    ),
}

/// Returned when parsing a string that is not in the closed catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownErrorCode(pub String);

impl fmt::Display for UnknownErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown error code: {}", self.0)
    }
}

impl std::error::Error for UnknownErrorCode {}

#[cfg(test)]
mod tests {
    use super::ErrorCode;
    use std::collections::HashSet;
    use std::str::FromStr;

    /// Transcribed from docs/specs/login-system.md §7.3. This is the
    /// independent source of truth for the closed catalog.
    const EXPECTED: &[(&str, u16)] = &[
        ("invalid_json", 400),
        ("unknown_field", 400),
        ("unsupported_media_type", 415),
        ("method_not_allowed", 405),
        ("payload_too_large", 413),
        ("validation_failed", 422),
        ("invalid_sort", 422),
        ("page_size_too_large", 422),
        ("empty_update", 422),
        ("invalid_time_range", 422),
        ("time_range_too_large", 422),
        ("invalid_credentials", 401),
        ("access_token_invalid", 401),
        ("session_invalid", 401),
        ("refresh_token_invalid", 401),
        ("refresh_reuse_detected", 401),
        ("account_disabled", 403),
        ("temporary_password_expired", 403),
        ("login_locked", 429),
        ("rate_limited", 429),
        ("password_change_required", 403),
        ("csrf_failed", 403),
        ("reauth_required", 403),
        ("invalid_current_password", 422),
        ("forbidden", 403),
        ("out_of_scope", 403),
        ("self_action_forbidden", 403),
        ("role_immutable", 403),
        ("permission_subset_violation", 403),
        ("not_found", 404),
        ("identifier_taken", 409),
        ("role_name_taken", 409),
        ("role_in_use", 409),
        ("last_super_admin", 409),
        ("password_policy_violation", 422),
        ("password_unchanged", 422),
        ("unknown_role", 422),
        ("empty_permissions", 422),
        ("invalid_permission", 422),
        ("internal_error", 500),
        ("service_unavailable", 503),
    ];

    #[test]
    fn catalog_exactly_matches_the_spec() {
        assert_eq!(ErrorCode::ALL.len(), EXPECTED.len());

        for (code, expected) in ErrorCode::ALL.iter().zip(EXPECTED) {
            assert_eq!(code.as_str(), expected.0, "code mismatch");
            assert_eq!(
                code.status(),
                expected.1,
                "status mismatch for {}",
                expected.0
            );
        }
    }

    #[test]
    fn codes_are_unique() {
        let unique: HashSet<_> = ErrorCode::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(unique.len(), ErrorCode::ALL.len());
    }

    #[test]
    fn contract_strings_are_stable() {
        for code in ErrorCode::ALL {
            let name = code.as_str();
            assert_eq!(code.type_uri(), format!("urn:rs-admin:problem:{name}"));
            assert!(!code.title().is_empty(), "{name} has no title");
            assert!(!code.detail().is_empty(), "{name} has no detail");
            assert!(code.status() >= 400 && code.status() <= 599);
            assert_eq!(ErrorCode::from_str(name).unwrap(), code);
        }
    }

    #[test]
    fn unknown_codes_are_rejected() {
        assert!(ErrorCode::from_str("password_too_short").is_err());
        assert!(ErrorCode::from_str("").is_err());
    }
}
