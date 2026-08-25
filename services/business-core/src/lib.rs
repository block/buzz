#![forbid(unsafe_code)]

pub mod api;
pub mod b2;
pub mod b3;
pub mod b4;
mod bootstrap;
pub mod config;
pub mod master_data;
pub mod master_data_api;
pub mod model;
pub mod numbering;
pub mod numbering_api;
pub mod product_master;
pub mod product_master_api;
pub mod s1;
pub mod security;
pub mod store;

pub use api::{router, AppState};
pub use config::Config;
pub use store::PgStore;
