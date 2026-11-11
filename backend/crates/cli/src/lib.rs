//! Offline management and scheduled operations.
//!
//! `bootstrap-super-admin` creates or promotes the first super administrator;
//! `recover-super-admin` is the break-glass path for an existing super
//! administrator; `record-key-rotation` writes the `crypto_key_rotated` audit
//! event after an operator rotates key material. `maintenance` applies
//! retention cleanup and `verify-audit-chain` verifies a restored cluster for
//! the restore drill. They reuse
//! `domain`/`store`/`security`/`audit`/`config` and are never reachable over
//! HTTP (spec §2.2, ADR-0003 §6, ADR-0008 §3.5 and §7).

pub mod args;
pub mod bootstrap;
mod credentials;
pub mod error;
pub mod input;
pub mod key_rotation;
pub mod maintenance;
pub mod recover;
pub mod runtime;
pub mod services;
pub mod verify;

pub use args::{
    ArgError, BootstrapArgs, KeyRotationArgs, ParseOutcome, RecoverArgs, parse_bootstrap,
    parse_record_key_rotation, parse_recover,
};
pub use bootstrap::{BootstrapInput, BootstrapOutcome, bootstrap_super_admin};
pub use error::CliError;
pub use input::{InputError, confirm_break_glass, read_password};
pub use key_rotation::{
    KeyRotationInput, KeyRotationOutcome, KeyType, RotationReason, record_key_rotation,
};
pub use maintenance::{MaintenanceRun, MaintenanceStatus, run_maintenance, run_maintenance_at};
pub use recover::{RecoverInput, RecoverOutcome, recover_super_admin};
pub use services::CliServices;
pub use verify::{RestoreVerification, RestoreVerifyError, verify_restored_database};
