//! Record a completed application-key rotation on the audit chain
//! (ADR-0005 §8.4, ADR-0008 §3.5).
//!
//! ```sh
//! record-key-rotation --key-type jwt --old-kid prod-2026-11 \
//!   --new-kid prod-2027-01 --reason scheduled
//! ```

use rs_admin_cli::runtime::{ensure_schema, store_pool_settings};
use rs_admin_cli::{
    KeyRotationInput, ParseOutcome, parse_record_key_rotation, record_key_rotation,
};
use rs_admin_config::{AuditChainConfig, DatabaseConfig};
use rs_admin_store::Database;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("record-key-rotation: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = match parse_record_key_rotation(std::env::args().skip(1))? {
        ParseOutcome::Parsed(arguments) => arguments,
        ParseOutcome::Help(usage) => {
            println!("{usage}");
            return Ok(());
        }
    };

    let database_config = DatabaseConfig::from_env()?;
    let audit_chain = AuditChainConfig::from_env()?;
    let database = Database::connect(
        database_config.url(),
        &store_pool_settings(&database_config.pool),
    )
    .await?;
    ensure_schema(&database).await?;

    let outcome = record_key_rotation(
        database.pool(),
        &audit_chain.keys,
        KeyRotationInput {
            key_type: arguments.key_type,
            old_kid: &arguments.old_kid,
            new_kid: &arguments.new_kid,
            reason: arguments.reason,
        },
    )
    .await;

    database.close().await;
    let outcome = outcome?;

    println!("Key rotation recorded.");
    println!("  audit id:   {}", outcome.audit_id);
    println!("  chain seq:  {}", outcome.chain_seq);
    println!("  key type:   {}", arguments.key_type.as_str());
    println!("  old key id: {}", arguments.old_kid);
    println!("  new key id: {}", arguments.new_kid);
    println!("  reason:     {}", arguments.reason.as_str());

    Ok(())
}
