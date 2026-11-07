//! Classification of user identity uniqueness conflicts.
//!
//! Handlers map [`IdentifierConflict`] to the stable API code
//! `identifier_taken` (spec §7.3) instead of inspecting SQL errors themselves.

/// Constraint names from the identity migration.
const USERS_USERNAME_UQ: &str = "users_username_uq";
const USERS_EMAIL_UQ: &str = "users_email_uq";
const UNIQUE_VIOLATION: &str = "23505";

/// Which login identifier collided with an existing user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierConflict {
    Username,
    Email,
}

impl IdentifierConflict {
    /// Classify a SQLx error, or return `None` if it is not a uniqueness
    /// violation on one of the user identifier constraints.
    pub fn from_unique_violation(error: &sqlx::Error) -> Option<Self> {
        let sqlx::Error::Database(error) = error else {
            return None;
        };
        if error.code().as_deref() != Some(UNIQUE_VIOLATION) {
            return None;
        }

        match error.constraint()? {
            USERS_USERNAME_UQ => Some(Self::Username),
            USERS_EMAIL_UQ => Some(Self::Email),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IdentifierConflict;
    use sqlx::error::{DatabaseError, ErrorKind};
    use std::borrow::Cow;
    use std::fmt;

    #[derive(Debug)]
    struct FakeDatabaseError {
        code: &'static str,
        constraint: Option<&'static str>,
    }

    impl fmt::Display for FakeDatabaseError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fake database error")
        }
    }

    impl std::error::Error for FakeDatabaseError {}

    impl DatabaseError for FakeDatabaseError {
        fn message(&self) -> &str {
            "duplicate key value violates unique constraint"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn constraint(&self) -> Option<&str> {
            self.constraint
        }

        fn kind(&self) -> ErrorKind {
            ErrorKind::UniqueViolation
        }
    }

    fn database_error(code: &'static str, constraint: Option<&'static str>) -> sqlx::Error {
        sqlx::Error::Database(Box::new(FakeDatabaseError { code, constraint }))
    }

    #[test]
    fn unique_violations_map_to_the_colliding_identifier() {
        assert_eq!(
            IdentifierConflict::from_unique_violation(&database_error(
                "23505",
                Some("users_username_uq")
            )),
            Some(IdentifierConflict::Username)
        );
        assert_eq!(
            IdentifierConflict::from_unique_violation(&database_error(
                "23505",
                Some("users_email_uq")
            )),
            Some(IdentifierConflict::Email)
        );
    }

    #[test]
    fn other_constraints_and_sql_states_are_not_identifier_conflicts() {
        assert_eq!(
            IdentifierConflict::from_unique_violation(&database_error("23505", Some("users_pkey"))),
            None
        );
        assert_eq!(
            IdentifierConflict::from_unique_violation(&database_error(
                "23503",
                Some("users_username_uq")
            )),
            None
        );
        assert_eq!(
            IdentifierConflict::from_unique_violation(&database_error("23505", None)),
            None
        );
        assert_eq!(
            IdentifierConflict::from_unique_violation(&sqlx::Error::RowNotFound),
            None
        );
    }
}
