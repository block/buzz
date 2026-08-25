//! Business Core B4 management profitability and reporting.

mod adjustments;
pub mod allocation;
pub mod api;
mod common;
pub mod model;
mod projection;
mod reporting;

pub use adjustments::AdjustmentService;
pub use projection::ProfitProjectionService;
pub use reporting::ProfitReportingService;
