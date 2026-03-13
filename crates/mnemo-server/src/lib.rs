//! # mnemo-server
//!
//! HTTP REST server for Mnemo, built on [Axum](https://github.com/tokio-rs/axum).
//!
//! ## Architecture
//!
//! - **[`config`]** — Environment-driven configuration (`MnemoConfig`).
//! - **[`state`]** — Shared application state (`AppState`) holding storage backends,
//!   LLM handles, webhook config, metrics, and feature flags.
//! - **[`routes`]** — All REST API handlers organized by domain: users, sessions,
//!   episodes, memory context, graph traversal, agents, webhooks, views,
//!   guardrails, API keys, regions, and operator endpoints.
//! - **[`middleware`]** — Auth middleware (API key validation, RBAC enforcement via
//!   `CallerContext`), request context propagation, and CORS/tracing layers.
//! - **[`dashboard`]** — Embedded SPA operator dashboard served at `/_/`.
//! - **[`grpc`]** — gRPC service implementations (memory, entity, edge) served
//!   on the same port via content-type routing (`application/grpc`).
//!
//! ## Storage
//!
//! The server connects to Redis (state/graph) and Qdrant (vector embeddings).
//! All storage traits are defined in `mnemo-core` and implemented in
//! `mnemo-storage`. The server accesses them through `AppState.state_store`
//! and `AppState.vector_store`.

pub mod config;
pub mod dashboard;
pub mod grpc;
pub mod middleware;
pub mod routes;
pub mod state;
