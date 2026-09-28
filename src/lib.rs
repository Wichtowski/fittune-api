//! FitTune API: workouts, activities, exercises and training analytics.

pub mod activities;
pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod equipment;
pub mod error;
pub mod exercises;
mod extract;
mod pagination;
pub mod photos;
pub mod places;
pub mod routines;
pub mod stats;
pub mod users;
mod validate;
pub mod workouts;

pub use app::{AppState, router};
