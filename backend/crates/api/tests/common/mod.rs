use rs_admin_api::AppState;
use rs_admin_store::{Database, PoolSettings};

/// State whose database is deliberately unreachable, so health/live and
/// middleware tests never touch a real server.
pub fn test_state() -> AppState {
    let database = Database::connect_lazy(
        "postgres://rs_admin:supersecret@127.0.0.1:1/rs_admin",
        &PoolSettings::default(),
    )
    .expect("lazy pool");
    AppState::new(database)
}
