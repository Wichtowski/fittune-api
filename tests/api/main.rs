//! HTTP-level integration tests. Each `#[sqlx::test]` runs against a fresh, migrated database
//! created from `DATABASE_URL` (e.g. `postgres://fittune:fittune@localhost:5432/fittune`).

mod activities;
mod auth;
mod barcode;
mod catalog;
mod common;
mod exercise_media;
mod exercises;
#[cfg(feature = "dev-fixtures")]
mod fixtures;
mod friends;
mod health;
mod invites;
mod namespaces;
mod ocr;
mod off_import;
mod photos;
mod places;
mod routines;
mod stats;
mod workouts;
