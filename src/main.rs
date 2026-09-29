use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpStream},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use fittune_api::{
    AppState,
    auth::{
        session,
        signup::{self, NewUser},
    },
    config::{Config, LogFormat},
    db,
    error::ApiError,
    exercises::media,
    router,
    users::{self, model::Role},
};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: fittune-api [serve | migrate | healthcheck | create-admin <username> <email> | grant-admin <username-or-email> | import-off <openfoodfacts.csv>]";

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
        ["create-admin", username, email] => create_admin(username, email).await,
        ["grant-admin", login] => grant_admin(login).await,
        ["import-off", path] => import_off(path).await,
        #[cfg(feature = "dev-fixtures")]
        ["seed-dev", flags @ ..] => seed_dev::run(flags).await,
        _ => bail!("{USAGE}"),
    }
}

/// Loads Open Food Facts' CSV export into `off_products`, replacing what was there
async fn import_off(path: &str) -> Result<()> {
    let config = Config::from_env()?;
    init_tracing(config.log_format);
    let pool = db::connect(&config.database_url, 2).await?;
    db::migrate(&pool).await?;
    let started = std::time::Instant::now();
    let summary =
        fittune_api::health::off::import::import_file(&pool, std::path::Path::new(path)).await?;
    let skipped = |skip| summary.skipped.get(&skip).copied().unwrap_or(0);
    use fittune_api::health::off::row::Skip;
    println!(
        "Imported {} products ({} without nutrition) from {} lines in {:.0?}; skipped: {} bad barcode, {} without a name, {} malformed; {} duplicate listings merged",
        summary.stored,
        summary.without_nutrition,
        summary.read,
        started.elapsed(),
        skipped(Skip::Barcode),
        skipped(Skip::Name),
        skipped(Skip::Malformed),
        summary.kept - summary.stored,
    );
    Ok(())
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

    let state = AppState::new(pool.clone(), config);
    spawn_catalog_media_backfill(&state);
    let app = router(state);
    // Connection info gives rate limiting a client address when no proxy header is configured
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
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

/// Copies catalog photos missing from storage once per start, without delaying requests.
fn spawn_catalog_media_backfill(state: &AppState) {
    let Some(store) = state.photos.clone() else {
        tracing::info!("photo storage is not configured, skipping the catalog photo backfill");
        return;
    };
    let db = state.db.clone();
    tokio::spawn(async move {
        let result = match media::HttpFetch::new() {
            Ok(fetch) => media::backfill(&db, store.as_ref(), &fetch).await,
            Err(err) => Err(err),
        };
        match result {
            Ok(summary) => tracing::info!(
                stored = summary.stored,
                present = summary.present,
                failed = summary.failed,
                "catalog photo backfill finished"
            ),
            Err(err) => tracing::warn!(error = format!("{err:#}"), "catalog photo backfill failed"),
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

/// Operator bootstrap for a closed installation: creates an admin with the same validation and
/// hashing as registration. The password is read from the terminal without echo (or from stdin
/// when piped), so it never appears in shell history, process lists or logs
async fn create_admin(username: &str, email: &str) -> Result<()> {
    let password = read_new_password()?;
    let user = match NewUser::validate(username, email, password, None, None) {
        Ok(user) => user,
        Err(err) => bail!("{}", describe(err)),
    };

    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url, 1).await?;
    db::migrate(&pool).await?;
    let mut tx = pool.begin().await?;
    let id = match signup::create(&mut tx, user, Role::Admin, None).await {
        Ok(id) => id,
        Err(err) => bail!("{}", describe(err)),
    };
    tx.commit().await?;
    println!("created admin {username} ({id})");
    Ok(())
}

fn read_new_password() -> Result<String> {
    if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        let password = rpassword::prompt_password("Password: ")?;
        if rpassword::prompt_password("Repeat password: ")? != password {
            bail!("passwords do not match");
        }
        Ok(password)
    } else {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        Ok(line.trim_end_matches(['\r', '\n']).to_owned())
    }
}

fn describe(err: ApiError) -> String {
    match err {
        ApiError::Validation { fields, .. } => fields
            .into_iter()
            .map(|(field, message)| format!("{field}: {message}"))
            .collect::<Vec<_>>()
            .join("; "),
        other => other.to_string(),
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

/// `seed-dev`: local fixtures, only in builds with the `dev-fixtures` feature.
#[cfg(feature = "dev-fixtures")]
mod seed_dev {
    use anyhow::{Context, Result, bail};
    use chrono::{NaiveDate, Utc};
    use fittune_api::{
        AppState,
        config::{Config, Registration},
        db,
        fixtures::{self, guard},
    };
    use sqlx::postgres::PgPoolOptions;

    const USAGE: &str = "usage: fittune-api seed-dev [--reset | --drop] [--base-date YYYY-MM-DD]";
    const BASE_DATE_VAR: &str = "FITTUNE_FIXTURES_BASE_DATE";

    #[derive(PartialEq)]
    enum Mode {
        /// Create or update the fixtures
        Seed,
        /// Delete the fixture accounts first, leaving other accounts alone
        Reset,
        /// Recreate the whole fixture database first
        Drop,
    }

    pub async fn run(flags: &[&str]) -> Result<()> {
        let mut mode = Mode::Seed;
        let mut base_date = env(BASE_DATE_VAR);
        let mut flags = flags.iter();
        while let Some(flag) = flags.next() {
            match *flag {
                "--reset" if mode == Mode::Seed => mode = Mode::Reset,
                "--drop" if mode == Mode::Seed => mode = Mode::Drop,
                "--base-date" => base_date = Some((*flags.next().context(USAGE)?).to_owned()),
                _ => bail!("{USAGE}"),
            }
        }
        let now = Utc::now();
        let base_date = match base_date {
            Some(date) => {
                NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").with_context(|| {
                    format!("the base date must look like 2026-09-28, got `{date}`")
                })?
            }
            None => now.date_naive(),
        };
        let clock = fixtures::Clock::new(base_date, now)?;

        let target = guard::check(
            env(guard::ENV_VAR).as_deref(),
            env(guard::DATABASE_URL_VAR).as_deref(),
            env("DATABASE_URL").as_deref(),
        )?;
        fixtures::prepare_database(&target, mode == Mode::Drop).await?;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(target.options.clone())
            .await
            .with_context(|| format!("failed to connect to {}", target.database))?;
        db::migrate(&pool).await?;

        let mut config = Config::from_env_with_database(String::new())?;
        // Like production, so the fixture accounts sign up with invites
        config.registration = Registration::InviteOnly;
        let state = AppState::new(pool.clone(), config);

        if mode == Mode::Reset {
            let removed = fixtures::reset(&state).await?;
            println!("Removed {removed} fixture accounts and everything they owned");
        }
        let summary = fixtures::seed(&state, clock).await?;
        pool.close().await;

        println!(
            "Seeded {} with base date {base_date}: {} new accounts, {} routines, {} workouts, {} activities",
            target.database,
            summary.accounts_created,
            summary.routines,
            summary.workouts,
            summary.activities,
        );
        if !summary.kept_workouts.is_empty() {
            println!(
                "Kept {} workouts the app saved since the last run: {}",
                summary.kept_workouts.len(),
                summary.kept_workouts.join(", ")
            );
        }
        println!(
            "\nFixture accounts (development only), password {}",
            fixtures::PASSWORD
        );
        for account in &fixtures::ACCOUNTS {
            println!("  {:<22} {}", account.email(), account.summary);
        }
        if let Some(code) = summary.active_invite_code {
            println!("\nActive invite code (shown only once): {code}");
        }
        println!("\nServe it with `make run-fixtures`, then sign in from the app");
        Ok(())
    }

    fn env(key: &str) -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}
