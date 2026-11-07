use anyhow::Result;
use rs_admin_api::store_pool_settings;
use rs_admin_config::DatabaseConfig;
use rs_admin_store::Database;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    match run().await {
        Ok(()) => {
            println!("migrations applied");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("migration failed: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let config = DatabaseConfig::from_env()?;
    let database = Database::connect(config.url(), &store_pool_settings(&config.pool)).await?;
    let result = database.migrate().await;
    database.close().await;
    result?;
    Ok(())
}
