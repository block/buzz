use crate::{b2::common::DomainError, model::AuthorizationSnapshot, store::PgStore};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn authorize(
    store: &PgStore,
    actor: Uuid,
    permission: &str,
    legal_entity: Option<Uuid>,
    warehouse: Option<Uuid>,
    supplier: Option<Uuid>,
    brand: Option<Uuid>,
    business_unit: Option<Uuid>,
) -> Result<AuthorizationSnapshot, DomainError> {
    let snapshot = store
        .snapshot(actor)
        .await
        .map_err(|_| DomainError::NotFoundOrForbidden)?;
    let allowed = snapshot.permission_keys.contains(permission)
        && legal_entity.is_none_or(|id| snapshot.scopes.legal_entity_ids.contains(&id))
        && warehouse.is_none_or(|id| snapshot.scopes.warehouse_ids.contains(&id))
        && supplier.is_none_or(|id| snapshot.scopes.supplier_ids.contains(&id))
        && brand.is_none_or(|id| snapshot.scopes.brand_ids.contains(&id))
        && business_unit.is_none_or(|id| snapshot.scopes.business_unit_ids.contains(&id));
    if allowed {
        Ok(snapshot)
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}
