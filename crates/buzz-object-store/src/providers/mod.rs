//! Concrete [`crate::ObjectStore`] implementations.
//!
//! Exactly one provider is constructed per process and shared by every domain
//! facade. Provider-specific vocabulary — ETags, generations, addressing
//! styles, credential chains — stays inside these modules.

pub mod gcs;
pub mod s3;
