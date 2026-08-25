use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::{collections::BTreeSet, str::FromStr};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    LegalEntity,
    LedgerBook,
    BusinessUnit,
    Department,
    UnitOfMeasure,
    ProductCategory,
    Brand,
    Warehouse,
    Customer,
    Supplier,
    Product,
    Sku,
    Salesperson,
}

impl ResourceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LegalEntity => "legal_entity",
            Self::LedgerBook => "ledger_book",
            Self::BusinessUnit => "business_unit",
            Self::Department => "department",
            Self::UnitOfMeasure => "unit_of_measure",
            Self::ProductCategory => "product_category",
            Self::Brand => "brand",
            Self::Warehouse => "warehouse",
            Self::Customer => "customer",
            Self::Supplier => "supplier",
            Self::Product => "product",
            Self::Sku => "sku",
            Self::Salesperson => "salesperson",
        }
    }
}

impl FromStr for ResourceType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "legal_entity" => Ok(Self::LegalEntity),
            "ledger_book" => Ok(Self::LedgerBook),
            "business_unit" => Ok(Self::BusinessUnit),
            "department" => Ok(Self::Department),
            "unit_of_measure" => Ok(Self::UnitOfMeasure),
            "product_category" => Ok(Self::ProductCategory),
            "brand" => Ok(Self::Brand),
            "warehouse" => Ok(Self::Warehouse),
            "customer" => Ok(Self::Customer),
            "supplier" => Ok(Self::Supplier),
            "product" => Ok(Self::Product),
            "sku" => Ok(Self::Sku),
            "salesperson" => Ok(Self::Salesperson),
            _ => Err("unsupported resourceType".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeDimension {
    LegalEntity,
    Warehouse,
    Customer,
    Supplier,
    Brand,
    BusinessUnit,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MasterDataRecord {
    pub resource_type: String,
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub status: String,
    pub legal_entity_id: Option<Uuid>,
    pub warehouse_id: Option<Uuid>,
    pub customer_id: Option<Uuid>,
    pub supplier_id: Option<Uuid>,
    pub brand_id: Option<Uuid>,
    pub business_unit_id: Option<Uuid>,
    pub version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct GroupProfile {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub base_currency: String,
    pub timezone: String,
    pub status: String,
    pub version: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataScopes {
    pub legal_entity_ids: BTreeSet<Uuid>,
    pub warehouse_ids: BTreeSet<Uuid>,
    pub customer_ids: BTreeSet<Uuid>,
    pub supplier_ids: BTreeSet<Uuid>,
    pub brand_ids: BTreeSet<Uuid>,
    pub business_unit_ids: BTreeSet<Uuid>,
}

impl DataScopes {
    pub fn permits(&self, resource: &MasterDataRecord) -> bool {
        resource
            .legal_entity_id
            .is_none_or(|id| self.legal_entity_ids.contains(&id))
            && resource
                .warehouse_id
                .is_none_or(|id| self.warehouse_ids.contains(&id))
            && resource
                .customer_id
                .is_none_or(|id| self.customer_ids.contains(&id))
            && resource
                .supplier_id
                .is_none_or(|id| self.supplier_ids.contains(&id))
            && resource
                .brand_id
                .is_none_or(|id| self.brand_ids.contains(&id))
            && resource
                .business_unit_id
                .is_none_or(|id| self.business_unit_ids.contains(&id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleSummary {
    pub id: Uuid,
    pub role_key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationSnapshot {
    pub enterprise_user_id: Uuid,
    pub roles: Vec<RoleSummary>,
    pub permission_keys: BTreeSet<String>,
    pub scopes: DataScopes,
    pub scope_version: i64,
    pub effective_scope_hash: String,
    pub evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccessCheckRequest {
    pub enterprise_user_id: Uuid,
    pub permission_key: String,
    pub resource_type: ResourceType,
    pub resource_id: Uuid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessCheckResponse {
    pub allowed: bool,
    pub scope_version: i64,
    pub effective_scope_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateQuery {
    pub action_code: String,
    pub resource_type: ResourceType,
    pub resource_id: Uuid,
    #[serde(default)]
    pub requester_user_id: Option<Uuid>,
    #[serde(default)]
    pub amount_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EligibleUser {
    pub enterprise_user_id: Uuid,
    pub display_name: String,
    pub role_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateResponse {
    pub action_code: String,
    pub candidates: Vec<EligibleUser>,
    pub minimum_approvers: Option<i16>,
    pub scope_version: i64,
    pub effective_scope_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleAssignmentRequest {
    pub enterprise_user_id: Uuid,
    pub role_id: Uuid,
    pub operation: GrantOperation,
    pub expected_authorization_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeMutationRequest {
    pub enterprise_user_id: Uuid,
    pub dimension: ScopeDimension,
    pub resource_id: Uuid,
    pub operation: GrantOperation,
    pub expected_authorization_revision: i64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantOperation {
    Grant,
    Revoke,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResponse {
    pub authorization_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapRequest {
    pub group: GroupProfileInput,
    pub legal_entities: Vec<LegalEntityInput>,
    pub ledger_books: Vec<LedgerBookInput>,
    pub business_units: Vec<BusinessUnitInput>,
    pub departments: Vec<DepartmentInput>,
    pub units_of_measure: Vec<UnitOfMeasureInput>,
    pub product_categories: Vec<ProductCategoryInput>,
    pub brands: Vec<NamedMasterInput>,
    pub warehouses: Vec<WarehouseInput>,
    pub customers: Vec<CustomerInput>,
    pub suppliers: Vec<PartyInput>,
    pub products: Vec<ProductInput>,
    pub skus: Vec<SkuInput>,
    pub salespeople: Vec<SalespersonInput>,
    pub roles: Vec<RoleInput>,
    pub user_roles: Vec<UserRoleInput>,
    pub scopes: Vec<ScopeGrantInput>,
    #[serde(default)]
    pub assignment_policies: Vec<AssignmentPolicyInput>,
    #[serde(default)]
    pub approval_policies: Vec<ApprovalPolicyInput>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupProfileInput {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub base_currency: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegalEntityInput {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub country_code: String,
    pub functional_currency: String,
    #[serde(default)]
    pub registration_number: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LedgerBookInput {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub code: String,
    pub name: String,
    pub currency: String,
    pub fiscal_year_start_month: i16,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedMasterInput {
    pub id: Uuid,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessUnitInput {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DepartmentInput {
    pub id: Uuid,
    pub business_unit_id: Uuid,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnitOfMeasureInput {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub precision_scale: i16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductCategoryInput {
    pub id: Uuid,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarehouseInput {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub business_unit_id: Uuid,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomerInput {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub business_unit_id: Uuid,
    pub code: String,
    pub name: String,
    pub credit_currency: String,
    pub credit_limit_minor: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartyInput {
    pub id: Uuid,
    pub legal_entity_id: Uuid,
    pub business_unit_id: Uuid,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductInput {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub category_id: Uuid,
    #[serde(default)]
    pub brand_id: Option<Uuid>,
    pub base_uom_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkuInput {
    pub id: Uuid,
    pub product_id: Uuid,
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub barcode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SalespersonInput {
    pub id: Uuid,
    pub enterprise_user_id: Uuid,
    pub business_unit_id: Uuid,
    #[serde(default)]
    pub department_id: Option<Uuid>,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleInput {
    pub id: Uuid,
    pub role_key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub permission_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserRoleInput {
    pub enterprise_user_id: Uuid,
    pub role_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeGrantInput {
    pub enterprise_user_id: Uuid,
    pub dimension: ScopeDimension,
    pub resource_id: Uuid,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AssignmentPolicyInput {
    pub action_code: String,
    pub required_permission: String,
    pub eligible_role_keys: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApprovalPolicyInput {
    pub action_code: String,
    pub required_permission: String,
    pub eligible_role_keys: Vec<String>,
    pub min_approvers: i16,
    #[serde(default)]
    pub allow_self_approval: bool,
    #[serde(default)]
    pub require_distinct_business_unit: bool,
    #[serde(default)]
    pub step_up_amount_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponse {
    pub group_id: Uuid,
    pub authorization_revision: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    pub code: &'static str,
    pub message: &'static str,
    pub trace_id: Uuid,
}
