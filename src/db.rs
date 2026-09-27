use std::time::Duration;

use anyhow::Context;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};

/// Migrations embedded into the binary, applied on startup.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect(url: &str, max_connections: u32) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
        .context("failed to connect to Postgres")
}

pub async fn migrate(pool: &PgPool) -> anyhow::Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .context("failed to run database migrations")
}
