//! Business Core B2 transactional sales and settlement modules.

pub mod api;
pub(crate) mod common;
mod inventory;
mod inventory_count;
mod inventory_count_api;
pub mod model;
mod return_disposition;
mod return_disposition_api;
mod returns;
mod sales;
mod settlement;

pub use common::DomainError;
pub use inventory::InventoryService;
pub use inventory_count::{
    CreateInventoryCount, InventoryCountDetail, InventoryCountOption, InventoryCountService,
    InventoryCountSummary, SubmitInventoryCount,
};
pub use return_disposition::{
    AcknowledgePurchaseReturn, DispatchPurchaseReturn, InspectSalesReturn, InspectionView,
    ReturnDispositionService,
};
pub use returns::{CreateReturn, ReturnOptions, ReturnService, ReturnSummary};
pub use sales::SalesService;
pub use settlement::SettlementService;
