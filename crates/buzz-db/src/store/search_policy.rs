//! Live `events.search_tsv` kind-0 policy inspection.
//!
//! Migration 0056 rewrites the generated search column only on an empty
//! `events` table; a populated database keeps its old expression until an
//! operator runs [`PROFILE_SEARCH_MAINTENANCE_SCRIPT`]. The relay's startup
//! warning and `buzz-admin profile-search-policy` both read the probe here so
//! they cannot disagree about whether that rewrite is still owed.

use serde::Serialize;
use sqlx::Row;

use crate::error::Result;
use crate::observability::{acquire_writer, WriterOperation};
use crate::Db;

/// Repo-relative path of the out-of-band rewrite a populated database needs
/// after 0056. Operator-facing messages name this so the fix is one copy-paste.
pub const PROFILE_SEARCH_MAINTENANCE_SCRIPT: &str =
    "scripts/maintenance/profile_search_text_fields.sql";

/// Repo-relative path of the procedure for running that rewrite.
pub const PROFILE_SEARCH_DEPLOYMENT_DOC: &str = "docs/profile-search-deployment.md";

/// How the live `events.search_tsv` generated column indexes kind-0 profiles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileSearchPolicy {
    /// The generated expression as `pg_get_expr` prints it.
    pub expression: String,
}

impl ProfileSearchPolicy {
    /// Whether kind 0 routes through `profile_search_tsv`, so a profile matches
    /// on its text fields and never on inline avatar bytes.
    pub fn indexes_profile_text_fields(&self) -> bool {
        self.expression.contains("profile_search_tsv(content)")
    }

    /// Whether the operator rewrite is still owed: kind 0 is still tokenized
    /// as whole JSON. Judged on the live expression alone. The row count says
    /// nothing about it: 0056 runs once, so a database that was populated then
    /// and emptied since keeps the old expression for every profile published
    /// from now on.
    pub fn rewrite_pending(&self) -> bool {
        !self.indexes_profile_text_fields()
    }
}

impl Db {
    /// Read the live kind-0 search policy without taking any lock beyond the
    /// catalog reads. `Ok(None)` when `events.search_tsv` does not exist, which
    /// only happens before the initial schema has been applied.
    pub async fn profile_search_policy(&self) -> Result<Option<ProfileSearchPolicy>> {
        let mut connection = acquire_writer(&self.pool, WriterOperation::Bootstrap).await?;
        let Some(row) = sqlx::query(
            "SELECT pg_get_expr(d.adbin, d.adrelid) AS expression \
             FROM pg_attrdef d \
             JOIN pg_attribute a ON a.attrelid = d.adrelid AND a.attnum = d.adnum \
             WHERE d.adrelid = to_regclass('events') \
               AND a.attname = 'search_tsv' \
               AND a.attgenerated = 's'",
        )
        .fetch_optional(&mut *connection)
        .await?
        else {
            return Ok(None);
        };
        let expression: String = row.try_get("expression")?;
        Ok(Some(ProfileSearchPolicy { expression }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POST_0056_EXPRESSION: &str = "CASE WHEN (kind = 0) THEN profile_search_tsv(content) \
         WHEN (kind = ANY (ARRAY[9, 40002, 45001, 45003])) THEN to_tsvector('simple'::regconfig, content) \
         ELSE NULL::tsvector END";
    const BLOCKLIST_EXPRESSION: &str = "CASE WHEN (kind = ANY (ARRAY[1059, 30179, 30300])) \
         THEN NULL::tsvector ELSE to_tsvector('simple'::regconfig, content) END";

    #[test]
    fn old_expression_is_pending() {
        let policy = ProfileSearchPolicy {
            expression: BLOCKLIST_EXPRESSION.to_owned(),
        };
        assert!(!policy.indexes_profile_text_fields());
        assert!(policy.rewrite_pending());
    }

    #[test]
    fn rewritten_expression_is_not_pending() {
        let policy = ProfileSearchPolicy {
            expression: POST_0056_EXPRESSION.to_owned(),
        };
        assert!(policy.indexes_profile_text_fields());
        assert!(!policy.rewrite_pending());
    }
}
