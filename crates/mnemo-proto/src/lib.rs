//! Protobuf/gRPC type definitions for the Mnemo memory API.
//!
//! This crate compiles `proto/mnemo/v1/memory.proto` into Rust types and
//! tonic service stubs. It provides both server traits (for `mnemo-server`)
//! and client stubs (for SDK/test use).
//!
//! # Services
//!
//! - [`memory_service_server`] — Context assembly, episode CRUD
//! - [`entity_service_server`] — Entity listing and lookup
//! - [`edge_service_server`] — Edge/fact query and lookup

pub mod proto {
    tonic::include_proto!("mnemo.v1");

    /// File descriptor set for gRPC server reflection.
    pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("mnemo_descriptor");
}

pub use proto::*;
