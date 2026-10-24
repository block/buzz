//! Mesh smoke harnesses.
//!
//! This crate deliberately carries no library code. It exists so the
//! mesh-llm SDK and native runtime can be a dev-dependency of the smoke
//! harnesses in `examples/` without entering `buzz-relay`'s test graph,
//! where it added the whole mesh-llm + skippy + rmcp compile to every relay
//! unit test build.
//!
//! See `examples/` for the harnesses and `scripts/ci-mesh-lifecycle-smoke.sh`
//! for the CI entry point.
