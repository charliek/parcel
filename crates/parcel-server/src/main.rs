//! Thin binary entrypoint: parse the CLI, load configuration, initialize
//! logging, and dispatch. All real logic lives in the library.

use anyhow::Context;
use clap::{Parser, Subcommand};

use parcel_server::config::Config;
use parcel_server::state::AppState;
use parcel_server::{db, gc, http, search};

#[derive(Parser)]
#[command(name = "parcel-server", version, about = "Parcel server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server (default).
    Serve,
    /// Print the OpenAPI document (what `openapi.json` must match).
    Openapi,
    /// Apply database migrations and exit (the Cloud Run migrate job).
    Migrate,
    /// Garbage-collect unreferenced blobs once and exit.
    Gc,
    /// Index every parcel whose page text isn't searchable yet (after a
    /// deploy that adds search; the server's own loop gets there too, a
    /// minute at a time) and exit.
    Reindex,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Needs no config, database or network.
    if let Some(Command::Openapi) = cli.command {
        println!(
            "{}",
            stridelabs_http::openapi::to_pretty_json(&parcel_server::openapi::spec())
        );
        return Ok(());
    }

    load_dotenv();
    let config = Config::from_env().context("loading configuration")?;
    stridelabs_observability::init_logging_with(
        config.log_format,
        "info,parcel_server=debug",
        &parcel_server::log_caps(),
    );

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => http::server::serve(config).await,
        Command::Migrate => {
            db::check_migration_url(&config.database_url)?;
            let state = AppState::new(config)?;
            db::migrate(&state.db).await?;
            tracing::info!("migrations applied");
            Ok(())
        }
        Command::Gc => {
            let state = AppState::new(config)?;
            let grace = state.config.gc_grace;
            let report = gc::run_once(&state, grace).await?;
            tracing::info!(?report, "gc completed");
            Ok(())
        }
        Command::Reindex => {
            let state = AppState::new(config)?;
            // No attempt cap: an operator asked for everything to be tried.
            let report = search::drain(&state, None, i16::MAX).await?;
            tracing::info!(
                indexed = report.indexed,
                failed = report.failed,
                "reindex completed"
            );
            Ok(())
        }
        Command::Openapi => unreachable!("handled before configuration was loaded"),
    }
}

/// Load `.env` in development. A parse error is reported WITHOUT the
/// offending text: dotenvy's error Display can quote the line, and `.env`
/// is where every secret lives (the spendwise-rs rule).
fn load_dotenv() {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(dotenvy::Error::LineParse(_, index)) => eprintln!(
            "warning: failed to parse .env (parser index {index}); contents withheld. \
             Check for an unclosed quote."
        ),
        Err(_) => eprintln!("warning: failed to load .env; details withheld"),
    }
}
