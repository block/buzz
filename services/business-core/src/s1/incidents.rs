use super::OperationsService;
use crate::{
    b2::{
        common::{authorize, begin_idempotent, finish_idempotent, request_hash},
        DomainError,
    },
    store::audit,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

const INCIDENT_READ_PERMISSION: &str = "management_report:read";
const INCIDENT_MANAGE_PERMISSION: &str = "management_report:manage_incidents";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncidentCommand {
    pub action: String,
    pub expected_version: i64,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub note: Option<String>,
}

impl OperationsService {
    pub async fn list_incidents(&self, actor: Uuid) -> Result<Value, DomainError> {
        let auth = authorize(
            &self.store,
            actor,
            INCIDENT_READ_PERMISSION,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let rows = sqlx::query(
            "SELECT i.*,u.display_name assignee_name FROM operating_report_incidents i LEFT JOIN enterprise_users u ON u.id=i.assignee_user_id WHERE i.scope_hash=$1 ORDER BY (i.review_status='resolved'),(i.condition_status='active') DESC,i.due_at,i.last_seen_at DESC LIMIT 100",
        )
        .bind(&auth.effective_scope_hash)
        .fetch_all(self.store.pool())
        .await?;
        let ids = rows
            .iter()
            .map(|row| row.get::<Uuid, _>("id"))
            .collect::<Vec<_>>();
        let event_rows = if ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query(
                "SELECT e.*,u.display_name actor_name FROM operating_report_incident_events e JOIN enterprise_users u ON u.id=e.actor_user_id WHERE e.incident_id=ANY($1) ORDER BY e.occurred_at DESC",
            )
            .bind(&ids)
            .fetch_all(self.store.pool())
            .await?
        };
        let mut events: HashMap<Uuid, Vec<Value>> = HashMap::new();
        for row in event_rows {
            let incident_id: Uuid = row.get("incident_id");
            let timeline = events.entry(incident_id).or_default();
            if timeline.len() < 20 {
                timeline.push(json!({
                    "id": row.get::<Uuid,_>("id"),
                    "eventType": row.get::<String,_>("event_type"),
                    "actorUserId": row.get::<Uuid,_>("actor_user_id"),
                    "actorName": row.get::<String,_>("actor_name"),
                    "occurredAt": row.get::<DateTime<Utc>,_>("occurred_at"),
                    "traceId": row.get::<Uuid,_>("trace_id"),
                    "payload": row.get::<Value,_>("payload")
                }));
            }
        }
        let now = Utc::now();
        let items = rows
            .into_iter()
            .map(|row| {
                let id: Uuid = row.get("id");
                let due_at: DateTime<Utc> = row.get("due_at");
                let status: String = row.get("review_status");
                json!({
                    "id": id,
                    "alertCode": row.get::<String,_>("alert_code"),
                    "severity": row.get::<String,_>("severity"),
                    "message": row.get::<String,_>("message"),
                    "evidencePath": row.get::<String,_>("evidence_path"),
                    "conditionStatus": row.get::<String,_>("condition_status"),
                    "reviewStatus": status,
                    "assigneeUserId": row.get::<Option<Uuid>,_>("assignee_user_id"),
                    "assigneeName": row.get::<Option<String>,_>("assignee_name"),
                    "dueAt": due_at,
                    "overdue": status != "resolved" && due_at < now,
                    "occurrenceCount": row.get::<i64,_>("occurrence_count"),
                    "firstSeenAt": row.get::<DateTime<Utc>,_>("first_seen_at"),
                    "lastSeenAt": row.get::<DateTime<Utc>,_>("last_seen_at"),
                    "clearedAt": row.get::<Option<DateTime<Utc>>,_>("cleared_at"),
                    "resolvedAt": row.get::<Option<DateTime<Utc>>,_>("resolved_at"),
                    "lastTraceId": row.get::<Uuid,_>("last_trace_id"),
                    "version": row.get::<i64,_>("version"),
                    "events": events.remove(&id).unwrap_or_default()
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "items": items,
            "dataAsOf": now,
            "source": "business-core-s1",
            "scopeVersion": auth.scope_version,
            "effectiveScopeHash": auth.effective_scope_hash,
            "boundary": "business_operations_only_not_financial_accounting"
        }))
    }

    pub async fn scan_incidents(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Value, DomainError> {
        authorize(
            &self.store,
            actor,
            INCIDENT_MANAGE_PERMISSION,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        let quality = self.data_quality(actor).await?;
        let scope_hash = quality["effectiveScopeHash"]
            .as_str()
            .ok_or_else(|| DomainError::Invalid("data quality scope is unavailable".into()))?;
        let alerts = quality["alerts"]
            .as_array()
            .cloned()
            .ok_or_else(|| DomainError::Invalid("data quality alerts are unavailable".into()))?;
        let hash = request_hash(&json!({"scopeHash":scope_hash}))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(value) = begin_idempotent::<Value>(
            &mut tx,
            actor,
            "operating_incident_scan",
            idempotency_key,
            &hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(value);
        }
        let mut codes = Vec::with_capacity(alerts.len());
        let mut created = 0_i64;
        let mut reopened = 0_i64;
        for item in alerts {
            let code = alert_text(&item, "code")?;
            let severity = alert_text(&item, "severity")?;
            let message = alert_text(&item, "message")?;
            let evidence_path = alert_text(&item, "evidencePath")?;
            codes.push(code.to_string());
            let existing = sqlx::query(
                "SELECT id,condition_status,review_status FROM operating_report_incidents WHERE scope_hash=$1 AND alert_code=$2 FOR UPDATE",
            )
            .bind(scope_hash)
            .bind(code)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(row) = existing {
                let id: Uuid = row.get("id");
                let was_cleared = row.get::<String, _>("condition_status") == "cleared";
                let was_resolved = row.get::<String, _>("review_status") == "resolved";
                sqlx::query("UPDATE operating_report_incidents SET severity=$2,message=$3,evidence_path=$4,condition_status='active',review_status=CASE WHEN $5 THEN 'open' ELSE review_status END,occurrence_count=occurrence_count+1,last_seen_at=now(),cleared_at=NULL,resolved_at=CASE WHEN $5 THEN NULL ELSE resolved_at END,last_trace_id=$6,version=version+1,updated_at=now() WHERE id=$1")
                    .bind(id).bind(severity).bind(message).bind(evidence_path).bind(was_resolved).bind(trace_id).execute(&mut *tx).await?;
                if was_cleared || was_resolved {
                    reopened += 1;
                    incident_event(
                        &mut tx,
                        id,
                        "reopened",
                        actor,
                        trace_id,
                        json!({"alertCode":code}),
                    )
                    .await?;
                }
            } else {
                let id = Uuid::new_v4();
                let due_at = Utc::now()
                    + if severity == "critical" {
                        Duration::hours(4)
                    } else {
                        Duration::hours(24)
                    };
                sqlx::query("INSERT INTO operating_report_incidents(id,scope_hash,alert_code,severity,message,evidence_path,due_at,created_by_user_id,last_trace_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
                    .bind(id).bind(scope_hash).bind(code).bind(severity).bind(message).bind(evidence_path).bind(due_at).bind(actor).bind(trace_id).execute(&mut *tx).await?;
                incident_event(
                    &mut tx,
                    id,
                    "detected",
                    actor,
                    trace_id,
                    json!({"alertCode":code,"severity":severity,"dueAt":due_at}),
                )
                .await?;
                created += 1;
            }
        }
        let cleared_rows = sqlx::query("SELECT id,alert_code FROM operating_report_incidents WHERE scope_hash=$1 AND condition_status='active' AND NOT (alert_code=ANY($2::text[])) FOR UPDATE")
            .bind(scope_hash).bind(&codes).fetch_all(&mut *tx).await?;
        for row in &cleared_rows {
            let id: Uuid = row.get("id");
            sqlx::query("UPDATE operating_report_incidents SET condition_status='cleared',cleared_at=now(),last_trace_id=$2,version=version+1,updated_at=now() WHERE id=$1")
                .bind(id).bind(trace_id).execute(&mut *tx).await?;
            incident_event(
                &mut tx,
                id,
                "condition_cleared",
                actor,
                trace_id,
                json!({"alertCode":row.get::<String,_>("alert_code")}),
            )
            .await?;
        }
        let result = json!({
            "createdCount": created,
            "reopenedCount": reopened,
            "clearedCount": cleared_rows.len(),
            "activeAlertCount": codes.len(),
            "traceId": trace_id
        });
        audit(
            &mut tx,
            trace_id,
            actor,
            "operating_incident.scan",
            "operating_report_incident",
            scope_hash,
            result.clone(),
        )
        .await?;
        finish_idempotent(
            &mut tx,
            actor,
            "operating_incident_scan",
            idempotency_key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn command_incident(
        &self,
        actor: Uuid,
        trace_id: Uuid,
        idempotency_key: &str,
        incident_id: Uuid,
        input: IncidentCommand,
    ) -> Result<Value, DomainError> {
        let auth = authorize(
            &self.store,
            actor,
            INCIDENT_MANAGE_PERMISSION,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        if input.note.as_ref().is_some_and(|note| note.len() > 500) {
            return Err(DomainError::Invalid(
                "note must not exceed 500 bytes".into(),
            ));
        }
        let hash = request_hash(&json!({"incidentId":incident_id,"command":input}))?;
        let mut tx = self.store.pool().begin().await?;
        if let Some(value) = begin_idempotent::<Value>(
            &mut tx,
            actor,
            "operating_incident_command",
            idempotency_key,
            &hash,
        )
        .await?
        {
            tx.commit().await?;
            return Ok(value);
        }
        let row = sqlx::query("SELECT condition_status,review_status,due_at,version FROM operating_report_incidents WHERE id=$1 AND scope_hash=$2 FOR UPDATE")
            .bind(incident_id).bind(&auth.effective_scope_hash).fetch_optional(&mut *tx).await?
            .ok_or(DomainError::NotFoundOrForbidden)?;
        if row.get::<i64, _>("version") != input.expected_version {
            return Err(DomainError::VersionConflict);
        }
        let condition: String = row.get("condition_status");
        let current: String = row.get("review_status");
        let mut next = current.clone();
        let mut assignee = None;
        let mut resolved_at = None;
        let mut due_at: DateTime<Utc> = row.get("due_at");
        let event_type = match input.action.as_str() {
            "claim" => {
                assignee = Some(actor);
                "claimed"
            }
            "acknowledge" if current == "open" => {
                next = "acknowledged".into();
                "acknowledged"
            }
            "start" if matches!(current.as_str(), "open" | "acknowledged") => {
                next = "in_progress".into();
                assignee = Some(actor);
                "started"
            }
            "resolve" if condition == "cleared" && current != "resolved" => {
                next = "resolved".into();
                resolved_at = Some(Utc::now());
                "resolved"
            }
            "set_due" => {
                due_at = input
                    .due_at
                    .filter(|value| *value > Utc::now())
                    .ok_or_else(|| DomainError::Invalid("dueAt must be in the future".into()))?;
                "due_changed"
            }
            "resolve" => {
                return Err(DomainError::Invalid(
                    "incident condition must be cleared before resolution".into(),
                ));
            }
            _ => return Err(DomainError::Invalid("invalid incident transition".into())),
        };
        let version = input.expected_version + 1;
        sqlx::query("UPDATE operating_report_incidents SET review_status=$2,assignee_user_id=COALESCE($3,assignee_user_id),due_at=$4,resolved_at=COALESCE($5,resolved_at),last_trace_id=$6,version=$7,updated_at=now() WHERE id=$1")
            .bind(incident_id).bind(&next).bind(assignee).bind(due_at).bind(resolved_at).bind(trace_id).bind(version).execute(&mut *tx).await?;
        let details = json!({"action":input.action,"fromStatus":current,"toStatus":next,"dueAt":due_at,"note":input.note,"version":version});
        incident_event(
            &mut tx,
            incident_id,
            event_type,
            actor,
            trace_id,
            details.clone(),
        )
        .await?;
        audit(
            &mut tx,
            trace_id,
            actor,
            "operating_incident.command",
            "operating_report_incident",
            &incident_id.to_string(),
            details,
        )
        .await?;
        let result = json!({"id":incident_id,"reviewStatus":next,"conditionStatus":condition,"assigneeUserId":assignee,"dueAt":due_at,"version":version,"traceId":trace_id});
        finish_idempotent(
            &mut tx,
            actor,
            "operating_incident_command",
            idempotency_key,
            &result,
        )
        .await?;
        tx.commit().await?;
        Ok(result)
    }
}

fn alert_text<'a>(alert: &'a Value, field: &str) -> Result<&'a str, DomainError> {
    alert[field]
        .as_str()
        .ok_or_else(|| DomainError::Invalid(format!("alert {field} is unavailable")))
}

async fn incident_event(
    tx: &mut Transaction<'_, Postgres>,
    incident_id: Uuid,
    event_type: &str,
    actor: Uuid,
    trace_id: Uuid,
    payload: Value,
) -> Result<(), DomainError> {
    sqlx::query("INSERT INTO operating_report_incident_events(id,incident_id,event_type,actor_user_id,trace_id,payload) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(Uuid::new_v4()).bind(incident_id).bind(event_type).bind(actor).bind(trace_id).bind(payload).execute(&mut **tx).await?;
    Ok(())
}
