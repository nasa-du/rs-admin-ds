//! Offline management commands.
//!
//! `bootstrap-super-admin` creates or promotes the first super administrator;
//! `recover-super-admin` is the break-glass path for an existing super
//! administrator. Both reuse `domain`/`store`/`security`/`audit`/`config` and
//! are never reachable over HTTP (spec §2.2, ADR-0003 §6).

pub mod args;
pub mod bootstrap;
mod credentials;
pub mod error;
pub mod input;
pub mod recover;
pub mod runtime;
pub mod services;

pub use args::{
    ArgError, BootstrapArgs, ParseOutcome, RecoverArgs, parse_bootstrap, parse_recover,
};
pub use bootstrap::{BootstrapInput, BootstrapOutcome, bootstrap_super_admin};
pub use error::CliError;
pub use input::{InputError, confirm_break_glass, read_password};
pub use recover::{RecoverInput, RecoverOutcome, recover_super_admin};
pub use services::CliServices;
