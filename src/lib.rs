//! FitTune API: training (FitTune) and nutrition (FitHealth) behind one account.

pub mod activities;
pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod equipment;
pub mod error;
pub mod exercises;
mod extract;
#[cfg(feature = "dev-fixtures")]
pub mod fixtures;
pub mod friends;
pub mod health;
pub mod invites;
mod pagination;
pub mod photos;
pub mod places;
pub mod rate_limit;
pub mod routines;
pub mod stats;
pub mod train;
pub mod users;
mod validate;
pub mod workouts;

pub use app::{AppState, router};
