mod routes;
mod sse;
mod state;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use sqlx::postgres::PgPoolOptions;
use sunmao_store::Store;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

#[derive(Parser, Debug)]
#[command(name = "sunmao-server", about = "sunmao task service")]
struct Args {
    /// Postgres connection string
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://sunmao@localhost/sunmao")]
    db: String,

    /// Listen address (D-18: default loopback)
    #[arg(long, default_value = "127.0.0.1:7420")]
    listen: String,

    /// Bind 0.0.0.0 (requires explicit opt-in)
    #[arg(long, default_value_t = false)]
    unsafe_expose: bool,

    /// Lease reaper interval seconds
    #[arg(long, default_value_t = 15)]
    reaper_interval_secs: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("sunmao_server=info".parse()?))
        .init();

    let args = Args::parse();
    let listen = if args.unsafe_expose {
        args.listen
            .replacen("127.0.0.1", "0.0.0.0", 1)
            .replacen("localhost", "0.0.0.0", 1)
    } else {
        args.listen
    };

    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&args.db)
        .await
        .with_context(|| format!("connect {}", args.db))?;

    Store::migrate(&pool).await.context("migrate")?;
    let store = Store::new(pool.clone());
    let (sse_tx, _) = tokio::sync::broadcast::channel(1024);
    let state = Arc::new(AppState {
        store: store.clone(),
        pool: pool.clone(),
        sse_tx: sse_tx.clone(),
    });

    // reaper
    {
        let store = store.clone();
        let interval = args.reaper_interval_secs;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval));
            loop {
                tick.tick().await;
                match store.reap_expired().await {
                    Ok(n) if n > 0 => tracing::info!(reaped = n, "lease reaper"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "reaper error"),
                }
            }
        });
    }

    // LISTEN fan-out
    {
        let db = args.db.clone();
        let sse_tx = sse_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = sse::listen_loop(&db, sse_tx).await {
                tracing::error!(error = %e, "LISTEN loop exited");
            }
        });
    }

    let app = routes::router(state);
    let addr: SocketAddr = listen.parse().context("listen addr")?;
    tracing::info!(%addr, "sunmao-server listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
