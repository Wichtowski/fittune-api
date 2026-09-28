//! Local development fixtures behind `fittune-api seed-dev`, compiled only with the
//! `dev-fixtures` feature.
//!
//! Fixture data is written through the real HTTP router with real sessions, so it passes the same
//! validation, ownership and authorization checks as the app. SQL is used directly only to create
//! the fixture admin (the same path as `create-admin`), to back-date the expired invite (the API
//! rightly refuses to), and to remove fixture accounts that can no longer sign in during a reset.

mod client;
pub mod guard;
mod plan;
mod seed;

use anyhow::{Context, Result};
use sqlx::{AssertSqlSafe, Connection, PgConnection};

pub use plan::{ACCOUNTS, Clock, PASSWORD};
pub use seed::{Summary, reset, seed};

/// Creates the fixture database if it is missing; with `drop`, recreates it empty first.
/// Only ever called with a [`guard::Target`], which is local and named `*_dev`.
pub async fn prepare_database(target: &guard::Target, drop: bool) -> Result<()> {
    let mut conn = PgConnection::connect_with(&target.maintenance)
        .await
        .context("failed to connect to the local Postgres server")?;
    // The guard allows only [a-z0-9_] names, so quoting the identifier is enough
    let name = format!("\"{}\"", target.database);
    if drop {
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&mut conn)
        .await
        .with_context(|| format!("failed to drop {}", target.database))?;
    }
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&target.database)
            .fetch_one(&mut conn)
            .await?;
    if !exists {
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&mut conn)
            .await
            .with_context(|| format!("failed to create {}", target.database))?;
    }
    conn.close().await?;
    Ok(())
}
