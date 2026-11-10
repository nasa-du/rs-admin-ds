//! Restore-drill verification against a restored database (ADR-0008 §5.6).
//!
//! ```sh
//! verify-audit-chain [--expected-migration-version VERSION]
//! ```
//!
//! Reads `DATABASE_URL`/`DATABASE_URL_FILE` and the audit chain keyring from
//! the environment, verifies the schema version, counts the key tables and
//! recomputes the HMAC chain. Prints a JSON summary; exits non-zero on any
//! mismatch. The output never contains the DSN, passwords or key material.

use rs_admin_cli::runtime::store_pool_settings;
use rs_admin_cli::verify_restored_database;
use rs_admin_config::{AuditChainConfig, DatabaseConfig};
use rs_admin_store::Database;
use std::process::ExitCode;

const USAGE: &str = "\
verify-audit-chain - verify a restored rs-admin database

Usage:
  verify-audit-chain [--expected-migration-version VERSION]

Options:
  --expected-migration-version VERSION
                        Fail unless _sqlx_migrations is exactly at VERSION
  -h, --help            Show this help

Prints a JSON object with the migration version, key table row counts and the
audit chain verification result. Exits non-zero when the schema or chain is
invalid.";

enum ParsedArgs {
    Help,
    Run {
        expected_migration_version: Option<i64>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("verify-audit-chain: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let expected_migration_version = match parse_args()? {
        ParsedArgs::Help => {
            println!("{USAGE}");
            return Ok(());
        }
        ParsedArgs::Run {
            expected_migration_version,
        } => expected_migration_version,
    };

    let database_config = DatabaseConfig::from_env()?;
    let audit_chain = AuditChainConfig::from_env()?;
    let pool_settings = store_pool_settings(&database_config.pool);
    let database = Database::connect(database_config.url(), &pool_settings).await?;

    let verification = verify_restored_database(
        database.pool(),
        &audit_chain.keys,
        expected_migration_version,
    )
    .await;
    database.close().await;

    let verification = verification?;
    println!("{}", serde_json::to_string_pretty(&verification)?);
    Ok(())
}

fn parse_args() -> Result<ParsedArgs, Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let mut expected_migration_version = None;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(ParsedArgs::Help),
            "--expected-migration-version" => {
                let value = arguments
                    .next()
                    .ok_or("--expected-migration-version requires a value")?;
                expected_migration_version = Some(
                    value
                        .parse::<i64>()
                        .map_err(|_| "--expected-migration-version must be an integer")?,
                );
            }
            _ => return Err("unknown argument".into()),
        }
    }

    Ok(ParsedArgs::Run {
        expected_migration_version,
    })
}
