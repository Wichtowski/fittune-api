use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use fittune_api::{
    AppState,
    auth::session,
    config::{Config, LogFormat},
    db, photos, router,
    users::{self, model::Role},
};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const USAGE: &str =
    "usage: fittune-api [serve | migrate | healthcheck | grant-admin <username-or-email>]";

#[tokio::main]
async fn main() -> Result<()> {
    // A missing .env file is normal in production, where variables come from the container.
    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["serve"] => serve(Config::from_env()?).await,
        ["migrate"] => {
            let config = Config::from_env()?;
            init_tracing(config.log_format);
            let pool = db::connect(&config.database_url, 1).await?;
            db::migrate(&pool).await
        }
        ["healthcheck"] => healthcheck(),
        ["grant-admin", login] => grant_admin(login).await,
        _ => bail!("{USAGE}"),
    }
}

async fn serve(config: Config) -> Result<()> {
    init_tracing(config.log_format);

    let pool = db::connect(&config.database_url, config.db_max_connections).await?;
    db::migrate(&pool).await?;
    spawn_session_janitor(pool.clone());

    let listener = TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;
    tracing::info!(addr = %config.bind_addr, version = %config.app_version, "fittune-api listening");

    let photo_store = config
        .photo_storage
        .as_ref()
        .map(|storage| Arc::new(photos::S3PhotoStore::new(storage)) as Arc<dyn photos::PhotoStore>);
    let app = router(AppState {
        db: pool.clone(),
        config: Arc::new(config),
        photos: photo_store,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    pool.close().await;
    tracing::info!("fittune-api stopped");
    Ok(())
}

fn init_tracing(format: LogFormat) {
    let filter = EnvFilter::try_from_env("FITTUNE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("fittune_api=info,tower_http=info,sqlx=warn,info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    match format {
        LogFormat::Json => builder
            .json()
            .flatten_event(true)
            .with_span_list(false)
            .init(),
        LogFormat::Pretty => builder.init(),
    }
}

/// Deletes expired sessions once an hour.
fn spawn_session_janitor(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
        loop {
            interval.tick().await;
            match session::purge_expired(&pool).await {
                Ok(0) => {}
                Ok(purged) => tracing::info!(purged, "purged expired sessions"),
                Err(err) => tracing::warn!(error = %err, "failed to purge expired sessions"),
            }
        }
    });
}

/// Resolves on Ctrl+C or SIGTERM (sent by `docker compose stop`).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to listen for Ctrl+C");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received, draining connections");
}

/// Container health probe: `GET /health` on the local port, exit non-zero unless it returns 200.
/// Implemented without an HTTP client so the runtime image needs no extra tooling.
fn healthcheck() -> Result<()> {
    let port = std::env::var("FITTUNE_BIND_ADDR")
        .ok()
        .and_then(|addr| addr.parse::<SocketAddr>().ok())
        .map_or(4733, |addr| addr.port());
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).context("connect failed")?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status_line = response.lines().next().unwrap_or_default();
    if status_line.split_whitespace().nth(1) == Some("200") {
        Ok(())
    } else {
        bail!("unhealthy: {status_line}")
    }
}

async fn grant_admin(login: &str) -> Result<()> {
    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url, 1).await?;
    match users::repo::set_role(&pool, login, Role::Admin).await? {
        Some(user) => {
            println!("granted admin role to {} <{}>", user.username, user.email);
            Ok(())
        }
        None => bail!("no user with username or email `{login}`"),
    }
}
