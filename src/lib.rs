//! takomo — central task store for AI agent fleets.
//!
//! Single binary: HTTP server (axum) over SQLite (WAL), plus `token` and
//! `project` admin subcommands that operate on the database directly.

pub mod api;
pub mod auth;
pub mod error;
pub mod ids;
pub mod mcp;
pub mod server;
pub mod store;
pub mod workflow;
