mod config;
mod gedcom;
mod mcp;

use std::{env, process};

use crate::config::Config;
use crate::gedcom::{GedcomStore, load_gedcom, load_store};
use crate::mcp::Server;

fn main() {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("failed to install tracing subscriber");

    let config_path = env::args()
        .nth(1)
        .or_else(|| env::var("GEDCOM_MCP_CONFIG").ok())
        .unwrap_or_else(|| "config.toml".into());
    let config = Config::from_path(&config_path).unwrap_or_else(|err| {
        eprintln!("Failed to load config from {}: {err}", config_path);
        process::exit(1);
    });

    tracing::info!(
        "Starting GEDCOM MCP server on {} using {}",
        config.bind_addr,
        config.gedcom_path.display()
    );

    let server = match (&config.gedcom_path, &config.persistence_path) {
        (ged_path, Some(store_path)) => {
            let server_store = match load_store(store_path) {
                Ok(store) => {
                    tracing::info!(
                        "Loaded persisted snapshot from {}; GEDCOM path available for reference: {}",
                        store_path.display(),
                        ged_path.display()
                    );
                    store
                }
                Err(err) => {
                    tracing::warn!(
                        "Failed to load snapshot from {} ({err}); falling back to GEDCOM at {}",
                        store_path.display(),
                        ged_path.display()
                    );
                    let gedcom_data = load_gedcom(ged_path).unwrap_or_else(|load_err| {
                        eprintln!(
                            "Failed to load GEDCOM data from {}: {load_err}",
                            ged_path.display()
                        );
                        process::exit(1);
                    });
                    GedcomStore::from_data(gedcom_data)
                }
            };

            Server::with_storage(server_store, store_path.clone())
        }
        (ged_path, None) => {
            tracing::info!(
                "Loading GEDCOM from {} (persistence disabled)",
                ged_path.display()
            );
            let gedcom_data = load_gedcom(ged_path).unwrap_or_else(|err| {
                eprintln!(
                    "Failed to load GEDCOM data from {}: {err}",
                    ged_path.display()
                );
                process::exit(1);
            });
            Server::new(Some(GedcomStore::from_data(gedcom_data)))
        }
    };
    tracing::info!(
        "Server initialized with GEDCOM data: listening for MCP messages on {} (stdin/stdout)",
        config.bind_addr
    );

    if let Err(err) = server.serve_lines(std::io::stdin().lock(), std::io::stdout().lock()) {
        eprintln!("Server loop exited with error: {err}");
        process::exit(1);
    }
}
