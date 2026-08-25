use crate::{b2::DomainError, model::AuthorizationSnapshot, store::PgStore};
use uuid::Uuid;

#[allow(clippy::too_many_arguments)]
pub async fn authorize(
    store: &PgStore,
    actor: Uuid,
    permission: &str,
    legal_entity: Option<Uuid>,
    warehouse: Option<Uuid>,
    customer: Option<Uuid>,
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
        && customer.is_none_or(|id| snapshot.scopes.customer_ids.contains(&id))
        && brand.is_none_or(|id| snapshot.scopes.brand_ids.contains(&id))
        && business_unit.is_none_or(|id| snapshot.scopes.business_unit_ids.contains(&id));
    if allowed {
        Ok(snapshot)
    } else {
        Err(DomainError::NotFoundOrForbidden)
    }
}

pub fn period(value: &str) -> Result<(), DomainError> {
    let month = value.get(5..).and_then(|part| part.parse::<u8>().ok());
    let valid = value.len() == 7
        && value.as_bytes()[4] == b'-'
        && value[..4].bytes().all(|byte| byte.is_ascii_digit())
        && month.is_some_and(|value| (1..=12).contains(&value));
    if valid {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "managementPeriod must use YYYY-MM".into(),
        ))
    }
}
