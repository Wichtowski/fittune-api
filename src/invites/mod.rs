//! Invite-only registration: admins issue single-use (or few-use) codes that expire

pub mod code;
pub mod model;
pub mod repo;
mod routes;

pub use routes::router;
