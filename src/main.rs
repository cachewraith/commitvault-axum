use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, bail};
use commitvault::{AppState, auth, build_app, config::Config, database, openapi};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

const USAGE: &str = "\
Usage: commitvault [COMMAND]

Commands:
  serve        Run migrations and start the HTTP server (default)
  migrate      Run pending database migrations and exit
  openapi      Print the OpenAPI document as JSON and exit
  healthcheck  Probe the local server's /api/v1/health endpoint (for container health checks)
";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `.env` is a development convenience; real deployments inject variables.
    dotenvy::dotenv().ok();

    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => serve().await,
        Some("migrate") => {
            init_tracing(false);
            let config = Config::from_env()?;
            let pool = database::connect(&config).await.context("failed to connect to the database")?;
            database::migrate(&pool).await.context("failed to run migrations")?;
            tracing::info!("migrations are up to date");
            Ok(())
        }
        Some("openapi") => {
            println!("{}", openapi::spec().to_pretty_json()?);
            Ok(())
        }
        Some("healthcheck") => healthcheck().await,
        Some("help" | "--help" | "-h") => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    init_tracing(config.env.is_production());

    let pool = database::connect(&config).await.context("failed to connect to the database")?;
    database::migrate(&pool).await.context("failed to run migrations")?;
    auth::bootstrap::ensure_admin(&pool, &config).await?;

    let cleanup =
        tokio::spawn(auth::session_store::PgSessionStore::new(pool.clone()).run_cleanup(Duration::from_secs(15 * 60)));

    let addr = SocketAddr::new(config.host, config.port);
    let app = build_app(AppState::new(pool.clone(), config));
    let listener = TcpListener::bind(addr).await.with_context(|| format!("failed to bind {addr}"))?;
    tracing::info!(%addr, "CommitVault API listening");

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    cleanup.abort();
    pool.close().await;
    tracing::info!("shut down cleanly");
    Ok(())
}

fn init_tracing(json: bool) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("commitvault=info,tower_http=info,sqlx=warn"));
    let registry = tracing_subscriber::registry().with(filter);
    if json {
        registry.with(tracing_subscriber::fmt::layer().json()).init();
    } else {
        registry.with(tracing_subscriber::fmt::layer()).init();
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received");
}

/// Minimal HTTP probe so the container image needs no curl/wget.
async fn healthcheck() -> anyhow::Result<()> {
    let port = std::env::var("APP_PORT").ok().and_then(|p| p.parse::<u16>().ok()).unwrap_or(3000);
    let probe = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        anyhow::Ok(response)
    };
    let response = tokio::time::timeout(Duration::from_secs(3), probe).await.context("health check timed out")??;
    if response.starts_with(b"HTTP/1.1 200") { Ok(()) } else { bail!("server is unhealthy") }
}
