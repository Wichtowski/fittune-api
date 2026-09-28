//! HTTP-level integration tests. Each `#[sqlx::test]` runs against a fresh, migrated database
//! created from `DATABASE_URL` (e.g. `postgres://fittune:fittune@localhost:5432/fittune`).

mod activities;
mod auth;
mod common;
mod exercises;
mod invites;
mod photos;
mod places;
mod routines;
mod stats;
mod workouts;
