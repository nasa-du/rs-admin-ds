//! Write the committed OpenAPI document.
//!
//! With a path argument the document is written to that file; with no
//! argument it is printed to stdout.
//!
//! ```sh
//! cargo run --quiet -p rs-admin-api --bin openapi -- backend/openapi.json
//! ```

use std::fs;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let json = rs_admin_api::openapi_document().to_pretty_json()?;

    match std::env::args_os().nth(1) {
        Some(path) => fs::write(PathBuf::from(path), format!("{json}\n"))?,
        None => println!("{json}"),
    }

    Ok(())
}
