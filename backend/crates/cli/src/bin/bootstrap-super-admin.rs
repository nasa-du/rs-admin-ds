//! Create or promote the first super administrator (ADR-0003 §6).
//!
//! ```sh
//! bootstrap-super-admin --email ops@example.com --username ops.admin
//! ```

use rs_admin_cli::runtime::{ensure_schema, store_pool_settings};
use rs_admin_cli::{
    BootstrapInput, CliServices, ParseOutcome, bootstrap_super_admin, parse_bootstrap,
    read_password,
};
use rs_admin_config::{AuditChainConfig, AuthSettings, DatabaseConfig};
use rs_admin_security::PasswordService;
use rs_admin_store::Database;
use secrecy::ExposeSecret;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bootstrap-super-admin: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = match parse_bootstrap(std::env::args().skip(1))? {
        ParseOutcome::Parsed(arguments) => arguments,
        ParseOutcome::Help(usage) => {
            println!("{usage}");
            return Ok(());
        }
    };

    let password = read_password(arguments.password_stdin)?;

    let database_config = DatabaseConfig::from_env()?;
    let auth = AuthSettings::from_env()?;
    let audit_chain = AuditChainConfig::from_env()?;
    let passwords = PasswordService::new(auth.password_hash_concurrency)?;
    let services =
        CliServices::from_parts(passwords, audit_chain.keys.clone(), auth.temp_password_ttl);

    let database = Database::connect(
        database_config.url(),
        &store_pool_settings(&database_config.pool),
    )
    .await?;
    ensure_schema(&database).await?;

    let outcome = bootstrap_super_admin(
        database.pool(),
        &services,
        BootstrapInput {
            email: &arguments.email,
            username: &arguments.username,
            password: &password,
        },
    )
    .await;

    database.close().await;
    let outcome = outcome?;

    let action = if outcome.created {
        "created"
    } else {
        "promoted"
    };
    println!("Super administrator {action}.");
    println!("  user id:            {}", outcome.user_id);
    println!("  username:           {}", outcome.username);
    println!("  email:              {}", outcome.email);
    println!(
        "  temporary password: {}",
        // Shown exactly once. Prompts never echo it, and it is not logged.
        password.expose_secret()
    );
    println!(
        "  expires at:         {}",
        outcome.temporary_password_expires_at.to_rfc3339()
    );
    if outcome.sessions_revoked > 0 {
        println!("  sessions revoked:   {}", outcome.sessions_revoked);
    }
    eprintln!("The temporary password is shown only once. The user must change it at first login.");

    Ok(())
}
