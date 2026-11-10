use anyhow::Result;
use rs_admin_api::{
    AppState, AuthServices, build_router, init_logging, install_metrics_recorder, metrics_router,
    shutdown_signal, spawn_db_pool_metrics, spawn_metrics_upkeep, store_pool_settings,
};
use rs_admin_config::Config;
use rs_admin_security::TrustedProxies;
use rs_admin_store::Database;
use std::process::ExitCode;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// Stop accepting new connections on signal, then force-close remaining
/// in-flight requests after this grace period.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    if let Err(error) = init_logging() {
        eprintln!("startup failed: {error}");
        return ExitCode::FAILURE;
    }

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "startup failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let config = Config::from_env()?;

    let database = Database::connect(
        config.database.url(),
        &store_pool_settings(&config.database.pool),
    )
    .await?;

    // Mirror the code-owned permission catalog and built-in roles before
    // serving traffic (spec §10 startup order). Migrations are a separate
    // deployment step; a missing schema fails startup rather than serving an
    // unauthorized-empty catalog.
    rs_admin_store::rbac::sync_catalog(database.pool()).await?;
    tracing::info!("permission catalog synchronized");

    if config.app_env.is_production() && config.trusted_proxy_cidrs.is_empty() {
        tracing::warn!(
            "TRUSTED_PROXY_CIDRS is empty; X-Forwarded-For is ignored and every request is \
             attributed to its direct peer"
        );
    }

    // Build all keyed services before serving: an unusable key is a startup
    // failure, never a per-request 500.
    let trusted_proxies = TrustedProxies::new(config.trusted_proxy_cidrs.clone());
    let auth = AuthServices::new(&config.security, &config.auth, trusted_proxies)?;
    tracing::info!("authentication services initialized");

    let metrics_handle = install_metrics_recorder()?;
    let upkeep = spawn_metrics_upkeep(metrics_handle.clone());
    let pool_metrics = spawn_db_pool_metrics(
        database.pool().clone(),
        config.database.pool.max_connections,
    );

    let app = build_router(
        AppState::with_auth(database.clone(), auth)
            .with_allowed_origins(config.origins.cors_allowed_origins.clone()),
    );
    let api_listener = TcpListener::bind(config.server.api_addr).await?;
    let metrics_listener = TcpListener::bind(config.server.metrics_addr).await?;

    tracing::info!("starting rs-admin-api");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let api_shutdown = shutdown_rx.clone();
    let api_task = tokio::spawn(async move {
        // Client IP throttling and audit depend on the TCP peer, so the
        // listener must publish `ConnectInfo<SocketAddr>` to handlers.
        axum::serve(
            api_listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(wait_for_shutdown(api_shutdown))
        .await
    });
    let metrics_task = tokio::spawn(async move {
        axum::serve(metrics_listener, metrics_router(metrics_handle))
            .with_graceful_shutdown(wait_for_shutdown(shutdown_rx))
            .await
    });

    let api_abort = api_task.abort_handle();
    let metrics_abort = metrics_task.abort_handle();
    let supervisor = tokio::spawn(async move {
        shutdown_signal().await;
        let _ = shutdown_tx.send(true);
        tokio::time::sleep(SHUTDOWN_GRACE).await;
        tracing::warn!("graceful shutdown timed out; aborting remaining connections");
        api_abort.abort();
        metrics_abort.abort();
    });

    let (api_result, metrics_result) = tokio::join!(api_task, metrics_task);
    supervisor.abort();
    finish(api_result)?;
    finish(metrics_result)?;

    upkeep.abort();
    pool_metrics.abort();
    database.close().await;
    tracing::info!("shutdown complete");
    Ok(())
}

async fn wait_for_shutdown(mut receiver: watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    let _ = receiver.changed().await;
}

/// A task cancelled by the shutdown grace timeout is a clean shutdown, not a
/// startup failure.
fn finish(result: Result<std::io::Result<()>, tokio::task::JoinError>) -> Result<()> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.into()),
        Err(error) if error.is_cancelled() => Ok(()),
        Err(error) => Err(error.into()),
    }
}
