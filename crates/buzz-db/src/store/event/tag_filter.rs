//! NIP-01 first-value matching shared by historical queries and exact COUNT.

use sqlx::{Postgres, QueryBuilder};

pub(super) fn push_predicates(
    qb: &mut QueryBuilder<Postgres>,
    col_prefix: &str,
    filters: &[(String, Vec<String>)],
) {
    for (name, values) in filters {
        if values.is_empty() {
            qb.push(" AND FALSE");
            continue;
        }
        // Use the existing tags GIN index to narrow candidates, then check
        // positions. Containment alone also matches a value in a namespace or
        // trailing marker, and even a reversed [value, name] array.
        let containments: Vec<serde_json::Value> = values
            .iter()
            .map(|value| serde_json::json!([[name, value]]))
            .collect();
        qb.push(format!(" AND {col_prefix}tags @> ANY("))
            .push_bind(containments)
            .push("::jsonb[]) AND EXISTS (SELECT 1 FROM jsonb_array_elements(")
            .push(format!("{col_prefix}tags"))
            .push(") AS matched_tag WHERE matched_tag->>0 = ")
            .push_bind(name.clone())
            .push(" AND matched_tag->>1 = ANY(")
            .push_bind(values.clone())
            .push("::text[]))");
    }
}

#[cfg(test)]
mod postgres_tests;
