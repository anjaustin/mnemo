//! Domain models for the Mnemo memory control plane.
//!
//! This module contains the core data types that define Mnemo's domain:
//! users, sessions, episodes, entities, edges (facts), and all higher-level
//! constructs built on top of them (narratives, goals, guardrails, regions,
//! views, API keys, agent identity, and webhook events).
//!
//! Each sub-module is self-contained with its own types, request/response
//! structs, validation logic, and unit tests.

pub mod agent;
pub mod api_key;
pub mod clarification;
pub mod classification;
pub mod context;
pub mod counterfactual;
pub mod digest;
pub mod edge;
pub mod entity;
pub mod episode;
pub mod goal;
pub mod guardrail;
pub mod narrative;
pub mod region;
pub mod session;
pub mod span;
pub mod user;
pub mod view;
pub mod webhook_event;
