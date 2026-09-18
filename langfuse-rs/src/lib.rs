//! Shared library surface for the Langfuse backend binaries.
//!
//! `bin/server.rs` runs the API plus the queue consumers in one process;
//! `bin/worker.rs` runs the consumers alone. Anything they must agree on lives
//! here rather than in each binary.

pub mod queues;
