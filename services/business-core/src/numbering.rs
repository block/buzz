use crate::{
    b2::common::{begin_idempotent, finish_idempotent, record, request_hash, DomainError},
    model::AuthorizationSnapshot,
    store::PgStore,
};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

const RECORD_TYPES: [&str; 15] = [
    "sales_order",
    "shipment",
    "receivable",
    "receipt",
    "opening",
    "purchase_order",
    "goods_receipt",
    "payable",
    "supplier_payment",
    "sales_return",
    "purchase_return",
    "inventory_count",
    "purchase_requisition",
    "profit_adjustment",
    "management_report",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NumberSegment {
    Fixed { value: String },
    Date { format: String },
    Scope,
    Sequence { width: u8 },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NumberingContext {
    pub legal_entity_id: Option<Uuid>,
    pub business_unit_id: Option<Uuid>,
}

impl NumberingContext {
    pub fn new(legal_entity_id: Uuid, business_unit_id: Option<Uuid>) -> Self {
        Self {
            legal_entity_id: Some(legal_entity_id),
            business_unit_id,
        }
    }
}

/// Allocates the next governed number inside the caller's transaction.
pub async fn allocate_number(
    tx: &mut Transaction<'_, Postgres>,
    record_type: &str,
    fallback_prefix: &str,
    aggregate_id: Uuid,
    context: NumberingContext,
) -> Result<String, DomainError> {
    crate::b2::common::next_number(tx, record_type, fallback_prefix, aggregate_id, context).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingRule {
    pub id: Uuid,
    pub record_type: String,
    pub name: String,
    pub segments: Vec<NumberSegment>,
    pub reset_period: String,
    pub scope_dimension: String,
    pub status: String,
    pub version: i64,
    pub updated_at: chrono::DateTime<Utc>,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingRuleList {
    pub items: Vec<NumberingRule>,
    pub can_manage: bool,
    pub data_as_of: chrono::DateTime<Utc>,
}

/// Read-only operating summary for the committed numbering ledger.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingLedger {
    pub summary: NumberingLedgerSummary,
    pub pools: Vec<NumberingLedgerPool>,
    pub recent_issuances: Vec<NumberingIssuance>,
    pub data_as_of: chrono::DateTime<Utc>,
}

/// Headline health indicators for numbering operations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingLedgerSummary {
    pub pool_count: i64,
    pub issued_last_30_days: i64,
    pub gap_count: i64,
    pub fallback_count: i64,
}

/// Current committed watermark and health for one scope and reset period.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingLedgerPool {
    pub record_type: String,
    pub rule_name: String,
    pub scope_dimension: String,
    pub scope_key: String,
    pub scope_label: String,
    pub period_key: String,
    pub current_value: i64,
    pub issued_count: i64,
    pub gap_count: i64,
    pub last_number: Option<String>,
    pub last_issued_at: Option<chrono::DateTime<Utc>>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// One number committed alongside its owning business aggregate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingIssuance {
    pub id: i64,
    pub record_type: String,
    pub aggregate_id: Uuid,
    pub rendered_number: String,
    pub source: String,
    pub scope_label: String,
    pub period_key: String,
    pub sequence_value: i64,
    pub gap_before: i64,
    pub gap_reason: Option<String>,
    pub issued_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveNumberingRule {
    pub name: String,
    pub segments: Vec<NumberSegment>,
    pub reset_period: String,
    pub scope_dimension: String,
    pub status: String,
    pub expected_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NumberingRuleCommandResult {
    pub id: Uuid,
    pub record_type: String,
    pub status: String,
    pub version: i64,
    pub preview: String,
    pub trace_id: Uuid,
    pub idempotent_replay: bool,
}

#[derive(Clone)]
pub struct NumberingRuleService {
    store: PgStore,
}

impl NumberingRuleService {
    pub fn new(store: PgStore) -> Self {
        Self { store }
    }

    async fn snapshot(
        &self,
        actor: Uuid,
        permission: &str,
    ) -> Result<AuthorizationSnapshot, DomainError> {
        let snapshot = self
            .store
            .snapshot(actor)
            .await
            .map_err(|_| DomainError::NotFoundOrForbidden)?;
        if snapshot.permission_keys.contains(permission) {
            Ok(snapshot)
        } else {
            Err(DomainError::NotFoundOrForbidden)
        }
    }

    pub async fn list(&self, actor: Uuid) -> Result<NumberingRuleList, DomainError> {
        let snapshot = self
            .snapshot(actor, "business_numbering_rules:read")
            .await?;
        let rows = sqlx::query(
            "SELECT id,record_type,name,segments,reset_period,scope_dimension,status,version,updated_at FROM business_numbering_rules ORDER BY array_position($1::text[],record_type)",
        )
        .bind(RECORD_TYPES.to_vec())
        .fetch_all(self.store.pool())
        .await?;
        let today = Utc::now().date_naive();
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let segments: Vec<NumberSegment> =
                serde_json::from_value(row.get::<Value, _>("segments"))?;
            let scope_dimension: String = row.get("scope_dimension");
            let preview_scope = match scope_dimension.as_str() {
                "legal_entity" => Some("LE01"),
                "business_unit" => Some("BU01"),
                _ => None,
            };
            items.push(NumberingRule {
                id: row.get("id"),
                record_type: row.get("record_type"),
                name: row.get("name"),
                preview: render_number(&segments, today, 42, preview_scope)?,
                segments,
                reset_period: row.get("reset_period"),
                scope_dimension,
                status: row.get("status"),
                version: row.get("version"),
                updated_at: row.get("updated_at"),
            });
        }
        Ok(NumberingRuleList {
            items,
            can_manage: snapshot
                .permission_keys
                .contains("business_numbering_rules:manage"),
            data_as_of: Utc::now(),
        })
    }

    /// Returns watermarks and committed issuance evidence without mutation controls.
    pub async fn ledger(&self, actor: Uuid) -> Result<NumberingLedger, DomainError> {
        self.snapshot(actor, "business_numbering_rules:read")
            .await?;
        let pool_rows = sqlx::query(
            r#"SELECT r.record_type,r.name,
                      CASE WHEN p.scope_key='global' THEN 'global'
                           WHEN le.id IS NOT NULL THEN 'legal_entity'
                           WHEN bu.id IS NOT NULL THEN 'business_unit'
                           ELSE r.scope_dimension
                      END scope_dimension,
                      p.scope_key,p.period_key,
                      p.current_value,p.updated_at,
                      CASE WHEN p.scope_key='global' THEN '全局'
                           WHEN le.id IS NOT NULL THEN le.code
                           WHEN bu.id IS NOT NULL THEN bu.code
                           ELSE '已删除主体'
                      END scope_label,
                      COALESCE(count(i.id),0)::bigint issued_count,
                      GREATEST(p.current_value-p.baseline_value-COALESCE(count(i.id),0),0)::bigint gap_count,
                      (array_agg(i.rendered_number ORDER BY i.sequence_value DESC) FILTER (WHERE i.id IS NOT NULL))[1] last_number,
                      max(i.issued_at) last_issued_at
               FROM business_numbering_sequence_pools p
               JOIN business_numbering_rules r ON r.id=p.rule_id
               LEFT JOIN business_legal_entities le ON le.id::text=p.scope_key
               LEFT JOIN business_units bu ON bu.id::text=p.scope_key
               LEFT JOIN business_numbering_issuances i ON i.rule_id=p.rule_id AND i.scope_key=p.scope_key AND i.period_key=p.period_key AND i.source='governed'
               GROUP BY r.record_type,r.name,r.scope_dimension,p.scope_key,p.period_key,p.current_value,p.baseline_value,p.updated_at,le.id,le.code,bu.id,bu.code
               ORDER BY p.updated_at DESC,r.record_type,p.scope_key,p.period_key DESC"#,
        )
        .fetch_all(self.store.pool())
        .await?;
        let pools = pool_rows
            .into_iter()
            .map(|row| NumberingLedgerPool {
                record_type: row.get("record_type"),
                rule_name: row.get("name"),
                scope_dimension: row.get("scope_dimension"),
                scope_key: row.get("scope_key"),
                scope_label: row.get("scope_label"),
                period_key: row.get("period_key"),
                current_value: row.get("current_value"),
                issued_count: row.get("issued_count"),
                gap_count: row.get("gap_count"),
                last_number: row.get("last_number"),
                last_issued_at: row.get("last_issued_at"),
                updated_at: row.get("updated_at"),
            })
            .collect::<Vec<_>>();
        let issuance_rows = sqlx::query(
            r#"WITH sequenced AS (
                 SELECT i.*,
                        GREATEST(i.sequence_value-COALESCE(lag(i.sequence_value) OVER (
                          PARTITION BY i.rule_id,i.scope_key,i.period_key,i.source
                          ORDER BY i.sequence_value,i.id
                        ),i.sequence_value-1)-1,0)::bigint gap_before
                 FROM business_numbering_issuances i
               )
               SELECT id,record_type,aggregate_id,rendered_number,source,
                      COALESCE(scope_code,CASE WHEN scope_key='global' THEN '全局' ELSE scope_key END) scope_label,
                      period_key,sequence_value,gap_before,
                      CASE WHEN gap_before>0 AND source='fallback'
                           THEN '安全回退序列不可回滚；中间事务未完成'
                           WHEN gap_before>0
                           THEN '受控序号池存在未登记消耗，请核查迁移或数据库操作'
                      END gap_reason,
                      issued_at
               FROM sequenced
               ORDER BY issued_at DESC,id DESC
               LIMIT 100"#,
        )
        .fetch_all(self.store.pool())
        .await?;
        let recent_issuances = issuance_rows
            .into_iter()
            .map(|row| NumberingIssuance {
                id: row.get("id"),
                record_type: row.get("record_type"),
                aggregate_id: row.get("aggregate_id"),
                rendered_number: row.get("rendered_number"),
                source: row.get("source"),
                scope_label: row.get("scope_label"),
                period_key: row.get("period_key"),
                sequence_value: row.get("sequence_value"),
                gap_before: row.get("gap_before"),
                gap_reason: row.get("gap_reason"),
                issued_at: row.get("issued_at"),
            })
            .collect::<Vec<_>>();
        let summary_row = sqlx::query(
            r#"WITH sequenced AS (
                 SELECT source,issued_at,
                        GREATEST(sequence_value-COALESCE(lag(sequence_value) OVER (
                          PARTITION BY rule_id,scope_key,period_key,source
                          ORDER BY sequence_value,id
                        ),sequence_value-1)-1,0)::bigint gap_before
                 FROM business_numbering_issuances
               )
               SELECT (SELECT count(*) FROM business_numbering_sequence_pools)::bigint pool_count,
                      count(*) FILTER (WHERE issued_at>=now()-interval '30 days')::bigint issued_last_30_days,
                      COALESCE(sum(gap_before),0)::bigint gap_count,
                      count(*) FILTER (WHERE source='fallback')::bigint fallback_count
               FROM sequenced"#,
        )
        .fetch_one(self.store.pool())
        .await?;
        Ok(NumberingLedger {
            summary: NumberingLedgerSummary {
                pool_count: summary_row.get("pool_count"),
                issued_last_30_days: summary_row.get("issued_last_30_days"),
                gap_count: summary_row.get("gap_count"),
                fallback_count: summary_row.get("fallback_count"),
            },
            pools,
            recent_issuances,
            data_as_of: Utc::now(),
        })
    }

    pub async fn save(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        record_type: &str,
        key: &str,
        input: &SaveNumberingRule,
    ) -> Result<NumberingRuleCommandResult, DomainError> {
        validate_record_type(record_type)?;
        validate_rule(record_type, input)?;
        self.snapshot(actor, "business_numbering_rules:manage")
            .await?;
        let hash = request_hash(&(record_type, input))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(mut replay) = begin_idempotent::<NumberingRuleCommandResult>(
            &mut tx,
            actor,
            "numbering_rule:save",
            key,
            &hash,
        )
        .await?
        {
            replay.idempotent_replay = true;
            tx.commit().await?;
            return Ok(replay);
        }
        let current = sqlx::query(
            "SELECT id,version,scope_dimension FROM business_numbering_rules WHERE record_type=$1 FOR UPDATE",
        )
        .bind(record_type)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DomainError::NotFoundOrForbidden)?;
        if current.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        let id: Uuid = current.get("id");
        let previous_scope_dimension: String = current.get("scope_dimension");
        let segments = serde_json::to_value(&input.segments)?;
        let row = sqlx::query(
            "UPDATE business_numbering_rules SET name=$2,segments=$3,status=$4,reset_period=$5,scope_dimension=$6,updated_by_user_id=$7 WHERE record_type=$1 RETURNING version",
        )
        .bind(record_type)
        .bind(input.name.trim())
        .bind(segments)
        .bind(&input.status)
        .bind(&input.reset_period)
        .bind(&input.scope_dimension)
        .bind(actor)
        .fetch_one(&mut *tx)
        .await?;
        let version: i64 = row.get("version");
        if previous_scope_dimension == input.scope_dimension {
            let current_period = period_key(&input.reset_period, Utc::now().date_naive())?;
            sqlx::query(
                "INSERT INTO business_numbering_sequence_pools(rule_id,scope_key,period_key,current_value,baseline_value) SELECT rule_id,scope_key,$2,max(current_value),max(current_value) FROM business_numbering_sequence_pools WHERE rule_id=$1 GROUP BY rule_id,scope_key ON CONFLICT(rule_id,scope_key,period_key) DO NOTHING",
            )
            .bind(id)
            .bind(current_period)
            .execute(&mut *tx)
            .await?;
        }
        let preview_scope = match input.scope_dimension.as_str() {
            "legal_entity" => Some("LE01"),
            "business_unit" => Some("BU01"),
            _ => None,
        };
        let preview = render_number(&input.segments, Utc::now().date_naive(), 42, preview_scope)?;
        record(
            &mut tx,
            trace_id,
            actor,
            "NUMBERING_RULE_SAVED",
            "numbering_rule_saved",
            "numbering_rule",
            id,
            json!({"recordType":record_type,"status":input.status,"version":version,"segments":input.segments,"resetPeriod":input.reset_period,"scopeDimension":input.scope_dimension,"preview":preview}),
        )
        .await?;
        let result = NumberingRuleCommandResult {
            id,
            record_type: record_type.into(),
            status: input.status.clone(),
            version,
            preview,
            trace_id,
            idempotent_replay: false,
        };
        finish_idempotent(&mut tx, actor, "numbering_rule:save", key, &result).await?;
        tx.commit().await?;
        Ok(result)
    }
}

pub fn render_number(
    segments: &[NumberSegment],
    date: NaiveDate,
    sequence: i64,
    scope_code: Option<&str>,
) -> Result<String, DomainError> {
    let mut output = String::new();
    for segment in segments {
        match segment {
            NumberSegment::Fixed { value } => output.push_str(value),
            NumberSegment::Date { format } => {
                output.push_str(&date.format(date_format(format)?).to_string())
            }
            NumberSegment::Scope => output.push_str(scope_code.ok_or_else(|| {
                DomainError::Invalid("scope code is required by the numbering rule".into())
            })?),
            NumberSegment::Sequence { width } => {
                output.push_str(&format!("{sequence:0width$}", width = usize::from(*width)))
            }
        }
    }
    if output.is_empty() || output.len() > 64 {
        return Err(DomainError::Invalid(
            "rendered number must contain 1 to 64 characters".into(),
        ));
    }
    Ok(output)
}

fn validate_record_type(value: &str) -> Result<(), DomainError> {
    if RECORD_TYPES.contains(&value) {
        Ok(())
    } else {
        Err(DomainError::Invalid(
            "unsupported numbering record type".into(),
        ))
    }
}

fn validate_rule(record_type: &str, input: &SaveNumberingRule) -> Result<(), DomainError> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(DomainError::Invalid(
            "rule name must contain 1 to 80 characters".into(),
        ));
    }
    if !matches!(input.status.as_str(), "active" | "disabled") {
        return Err(DomainError::Invalid(
            "status must be active or disabled".into(),
        ));
    }
    if !matches!(
        input.reset_period.as_str(),
        "never" | "yearly" | "monthly" | "daily"
    ) {
        return Err(DomainError::Invalid("unsupported reset period".into()));
    }
    if !matches!(
        input.scope_dimension.as_str(),
        "global" | "legal_entity" | "business_unit"
    ) {
        return Err(DomainError::Invalid("unsupported sequence scope".into()));
    }
    if (input.scope_dimension == "business_unit"
        && matches!(
            record_type,
            "opening" | "profit_adjustment" | "management_report"
        ))
        || (record_type == "management_report" && input.scope_dimension != "global")
    {
        return Err(DomainError::Invalid(
            "this record type does not have one authoritative scope at that dimension".into(),
        ));
    }
    if input.segments.len() < 2 || input.segments.len() > 8 {
        return Err(DomainError::Invalid(
            "numbering rule must contain 2 to 8 segments".into(),
        ));
    }
    let mut date_count = 0;
    let mut scope_count = 0;
    let mut sequence_count = 0;
    for segment in &input.segments {
        match segment {
            NumberSegment::Fixed { value } => {
                if value.is_empty()
                    || value.chars().count() > 24
                    || !value.chars().all(|character| {
                        character.is_ascii_alphanumeric()
                            || matches!(character, '-' | '_' | '/' | '.')
                    })
                {
                    return Err(DomainError::Invalid(
                        "fixed segments accept 1 to 24 letters, digits, hyphens, underscores, slashes, or dots"
                            .into(),
                    ));
                }
            }
            NumberSegment::Date { format } => {
                date_format(format)?;
                date_count += 1;
            }
            NumberSegment::Scope => scope_count += 1,
            NumberSegment::Sequence { width } => {
                if !(3..=10).contains(width) {
                    return Err(DomainError::Invalid(
                        "sequence width must be between 3 and 10".into(),
                    ));
                }
                sequence_count += 1;
            }
        }
    }
    if date_count > 1 || sequence_count != 1 {
        return Err(DomainError::Invalid(
            "a rule may contain one date segment and must contain exactly one sequence segment"
                .into(),
        ));
    }
    if (input.scope_dimension == "global" && scope_count != 0)
        || (input.scope_dimension != "global" && scope_count != 1)
    {
        return Err(DomainError::Invalid(
            "scoped sequence pools require exactly one scope code segment".into(),
        ));
    }
    let date_formats = input
        .segments
        .iter()
        .filter_map(|segment| match segment {
            NumberSegment::Date { format } => Some(format.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let reset_has_unique_date = match input.reset_period.as_str() {
        "never" => true,
        "yearly" => !date_formats.is_empty(),
        "monthly" => date_formats
            .iter()
            .any(|value| matches!(*value, "YYYYMM" | "YYYYMMDD" | "YYMM" | "YYMMDD")),
        "daily" => date_formats
            .iter()
            .any(|value| matches!(*value, "YYYYMMDD" | "YYMMDD")),
        _ => false,
    };
    if !reset_has_unique_date {
        return Err(DomainError::Invalid(
            "reset period requires a date segment with matching precision".into(),
        ));
    }
    let preview_scope = (input.scope_dimension != "global").then_some("SCOPE");
    render_number(&input.segments, Utc::now().date_naive(), 42, preview_scope).map(|_| ())
}

pub fn period_key(reset_period: &str, date: NaiveDate) -> Result<String, DomainError> {
    match reset_period {
        "never" => Ok("*".into()),
        "yearly" => Ok(date.format("%Y").to_string()),
        "monthly" => Ok(date.format("%Y%m").to_string()),
        "daily" => Ok(date.format("%Y%m%d").to_string()),
        _ => Err(DomainError::Invalid("unsupported reset period".into())),
    }
}

fn date_format(value: &str) -> Result<&'static str, DomainError> {
    match value {
        "YYYY" => Ok("%Y"),
        "YYYYMM" => Ok("%Y%m"),
        "YYYYMMDD" => Ok("%Y%m%d"),
        "YYMM" => Ok("%y%m"),
        "YYMMDD" => Ok("%y%m%d"),
        _ => Err(DomainError::Invalid("unsupported date format".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_ordered_fixed_date_and_sequence_segments() {
        let segments = vec![
            NumberSegment::Fixed {
                value: "SO-".into(),
            },
            NumberSegment::Date {
                format: "YYYYMMDD".into(),
            },
            NumberSegment::Scope,
            NumberSegment::Fixed { value: "-".into() },
            NumberSegment::Sequence { width: 5 },
        ];
        let date = match NaiveDate::from_ymd_opt(2026, 8, 22) {
            Some(value) => value,
            None => panic!("fixture date must be valid"),
        };
        assert_eq!(
            render_number(&segments, date, 42, Some("LE01"))
                .ok()
                .as_deref(),
            Some("SO-20260822LE01-00042")
        );
    }

    #[test]
    fn rejects_rules_without_exactly_one_sequence() {
        let input = SaveNumberingRule {
            name: "销售订单编码".into(),
            segments: vec![
                NumberSegment::Fixed {
                    value: "SO-".into(),
                },
                NumberSegment::Date {
                    format: "YYYYMM".into(),
                },
            ],
            reset_period: "monthly".into(),
            scope_dimension: "global".into(),
            status: "active".into(),
            expected_version: 1,
        };
        assert!(matches!(
            validate_rule("sales_order", &input),
            Err(DomainError::Invalid(_))
        ));
    }

    #[test]
    fn derives_stable_reset_period_keys() {
        let date = match NaiveDate::from_ymd_opt(2026, 8, 23) {
            Some(value) => value,
            None => panic!("fixture date must be valid"),
        };
        assert_eq!(period_key("never", date).ok().as_deref(), Some("*"));
        assert_eq!(period_key("yearly", date).ok().as_deref(), Some("2026"));
        assert_eq!(period_key("monthly", date).ok().as_deref(), Some("202608"));
        assert_eq!(period_key("daily", date).ok().as_deref(), Some("20260823"));
    }
}
