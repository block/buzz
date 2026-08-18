//! NIP-56 report (kind:1984) validation + persistence (Phase 1 contract).
//!
//! Reports are signals, never triggers (NIP-56): the relay persists them to
//! the tenant-scoped moderation queue and **never** auto-actions or fans them
//! out publicly.
//!
//! ## The pinned invariant (MOD, `docs/spec/MultiTenantRelay.tla`)
//! Report targets resolve under `tenant.community()` **only**:
//! - `e` target → event row looked up in this tenant; infer `channel_id`
//!   from it. Not found in-tenant ⇒ reject (never search other tenants).
//! - `x` blob target → tenant-scoped media reference `(community_id, sha256)`.
//!   A bare SHA-256 is shared across tenants and must not grant cross-tenant
//!   visibility.
//! - `p`-only target → community-local report about that pubkey in this
//!   tenant; implies nothing platform/global.
//!
//! Lane ownership: L3 (Perci) — including the `required_scope_for_kind` /
//! storage-suppression wiring in `ingest.rs`.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use buzz_db::moderation::{NewReport, ReportTarget};
use nostr::Event;

use crate::state::AppState;

/// NIP-56 report types accepted at ingest.
pub const REPORT_TYPES: &[&str] = &[
    "illegal",
    "nudity",
    "malware",
    "spam",
    "impersonation",
    "profanity",
    "other",
];

/// Validate a kind:1984 report and persist it to `moderation_reports`.
///
/// Rejections use client-safe `invalid:`/`restricted:` reasons. On success
/// the report is queued (idempotently, keyed by the signed event id) and the
/// event itself is **not** stored or broadcast as a regular event.
pub async fn handle_report_event(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> Result<(), String> {
    let parsed = parse_report(event)?;
    let reporter_pubkey = event.pubkey.to_bytes();

    let (target, channel_id) = match parsed.target {
        ParsedReportTarget::Event { event_id, .. } => {
            let stored = state
                .db
                .get_event_by_id(tenant.community(), &event_id)
                .await
                .map_err(|e| format!("error: database error resolving report target: {e}"))?
                .ok_or_else(|| "invalid: report target event not found".to_string())?;
            (ReportTarget::Event(event_id), stored.channel_id)
        }
        ParsedReportTarget::Blob { sha256, .. } => {
            let sha_hex = hex::encode(&sha256);
            // Known Phase-1 limitation: the media sidecar API does not expose a
            // cheap typed not-found vs transient-storage distinction here, so
            // all lookup failures surface as a missing blob to the reporter.
            state
                .media_storage
                .get_sidecar(tenant, &sha_hex)
                .await
                .map_err(|_| "invalid: report target blob not found".to_string())?;
            (ReportTarget::Blob(sha256), None)
        }
        ParsedReportTarget::Pubkey { pubkey } => (ReportTarget::Pubkey(pubkey), None),
    };

    state
        .db
        .insert_moderation_report(
            tenant.community(),
            NewReport {
                report_event_id: event.id.as_bytes(),
                reporter_pubkey: &reporter_pubkey,
                target,
                channel_id,
                report_type: parsed.report_type,
                note: report_note(event),
            },
        )
        .await
        .map_err(|e| format!("error: database error inserting report: {e}"))?;

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedReport<'a> {
    target: ParsedReportTarget,
    report_type: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedReportTarget {
    Event {
        event_id: Vec<u8>,
        /// Validation-shape only: NIP-56 requires the reported pubkey tag, but
        /// e-target author truth comes from the stored tenant event row.
        author_pubkey: Vec<u8>,
    },
    Blob {
        sha256: Vec<u8>,
        /// Validation-shape only: NIP-56 requires the reported pubkey tag, but
        /// blob authorship is not trusted from the reporter or inserted in v1.
        author_pubkey: Vec<u8>,
    },
    Pubkey {
        pubkey: Vec<u8>,
    },
}

fn parse_report(event: &Event) -> Result<ParsedReport<'_>, String> {
    let p_tags = collect_report_tags(event, "p");
    if p_tags.is_empty() {
        return Err("invalid: report must include a p tag".to_string());
    }
    if p_tags.len() > 1 {
        return Err("invalid: report must include exactly one p tag".to_string());
    }

    let reported_pubkey = decode_32_byte_hex(p_tags[0].value, "p tag pubkey")?;

    let e_tags = collect_report_tags(event, "e");
    let x_tags = collect_report_tags(event, "x");
    if !e_tags.is_empty() && !x_tags.is_empty() {
        return Err("invalid: report must target only one of e or x".to_string());
    }
    if e_tags.len() > 1 {
        return Err("invalid: report must include at most one e tag".to_string());
    }
    if x_tags.len() > 1 {
        return Err("invalid: report must include at most one x tag".to_string());
    }

    if let Some(tag) = e_tags.first() {
        let report_type = parse_report_type(tag.report_type)?;
        return Ok(ParsedReport {
            target: ParsedReportTarget::Event {
                event_id: decode_32_byte_hex(tag.value, "e tag event id")?,
                author_pubkey: reported_pubkey,
            },
            report_type,
        });
    }

    if let Some(tag) = x_tags.first() {
        let report_type = parse_report_type(tag.report_type)?;
        return Ok(ParsedReport {
            target: ParsedReportTarget::Blob {
                sha256: decode_32_byte_hex(tag.value, "x tag sha256")?,
                author_pubkey: reported_pubkey,
            },
            report_type,
        });
    }

    let p_tag = p_tags[0];
    let report_type = parse_report_type(p_tag.report_type)?;
    Ok(ParsedReport {
        target: ParsedReportTarget::Pubkey {
            pubkey: reported_pubkey,
        },
        report_type,
    })
}

#[derive(Debug, Clone, Copy)]
struct ReportTag<'a> {
    value: &'a str,
    report_type: Option<&'a str>,
}

fn collect_report_tags<'a>(event: &'a Event, tag_name: &str) -> Vec<ReportTag<'a>> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            if fields.len() >= 2 && fields[0] == tag_name {
                Some(ReportTag {
                    value: fields[1].as_str(),
                    report_type: fields.get(2).map(String::as_str),
                })
            } else {
                None
            }
        })
        .collect()
}

fn parse_report_type(value: Option<&str>) -> Result<&str, String> {
    let Some(value) = value else {
        return Err("invalid: report target tag missing report type".to_string());
    };
    if REPORT_TYPES.contains(&value) {
        Ok(value)
    } else {
        Err(format!("invalid: unsupported report type: {value}"))
    }
}

fn decode_32_byte_hex(value: &str, label: &str) -> Result<Vec<u8>, String> {
    if value.len() != 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid: malformed {label}"));
    }
    let bytes = hex::decode(value).map_err(|_| format!("invalid: malformed {label}"))?;
    if bytes.len() != 32 {
        return Err(format!("invalid: malformed {label}"));
    }
    Ok(bytes)
}

fn report_note(event: &Event) -> Option<&str> {
    if event.content.is_empty() {
        None
    } else {
        Some(event.content.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::KIND_REPORT;

    fn report_with_tags(tags: &[&[&str]]) -> Event {
        let keys = nostr::Keys::generate();
        let tags = tags
            .iter()
            .map(|tag| nostr::Tag::parse(tag.iter().copied()).unwrap())
            .collect::<Vec<_>>();
        nostr::EventBuilder::new(nostr::Kind::Custom(KIND_REPORT as u16), "note")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn hex32(byte: u8) -> String {
        hex::encode([byte; 32])
    }

    #[test]
    fn parses_event_report_type_from_e_tag_third_element() {
        let e = hex32(1);
        let p = hex32(2);
        let event = report_with_tags(&[&["p", &p], &["e", &e, "spam"]]);

        let parsed = parse_report(&event).unwrap();

        assert_eq!(parsed.report_type, "spam");
        assert_eq!(
            parsed.target,
            ParsedReportTarget::Event {
                event_id: vec![1; 32],
                author_pubkey: vec![2; 32],
            }
        );
    }

    #[test]
    fn parses_blob_report_type_from_x_tag_third_element() {
        let x = hex32(3);
        let p = hex32(4);
        let event = report_with_tags(&[&["p", &p], &["x", &x, "malware"]]);

        let parsed = parse_report(&event).unwrap();

        assert_eq!(parsed.report_type, "malware");
        assert_eq!(
            parsed.target,
            ParsedReportTarget::Blob {
                sha256: vec![3; 32],
                author_pubkey: vec![4; 32],
            }
        );
    }

    #[test]
    fn parses_pubkey_only_report_type_from_p_tag_third_element() {
        let p = hex32(5);
        let event = report_with_tags(&[&["p", &p, "impersonation"]]);

        let parsed = parse_report(&event).unwrap();

        assert_eq!(parsed.report_type, "impersonation");
        assert_eq!(
            parsed.target,
            ParsedReportTarget::Pubkey {
                pubkey: vec![5; 32]
            }
        );
    }

    #[test]
    fn rejects_reports_without_p_tag() {
        let e = hex32(1);
        let event = report_with_tags(&[&["e", &e, "spam"]]);

        assert_eq!(
            parse_report(&event).unwrap_err(),
            "invalid: report must include a p tag"
        );
    }

    #[test]
    fn rejects_unknown_report_type() {
        let p = hex32(1);
        let event = report_with_tags(&[&["p", &p, "phishing"]]);

        assert_eq!(
            parse_report(&event).unwrap_err(),
            "invalid: unsupported report type: phishing"
        );
    }

    #[test]
    fn rejects_event_and_blob_targets_together() {
        let p = hex32(1);
        let e = hex32(2);
        let x = hex32(3);
        let event = report_with_tags(&[&["p", &p], &["e", &e, "spam"], &["x", &x, "spam"]]);

        assert_eq!(
            parse_report(&event).unwrap_err(),
            "invalid: report must target only one of e or x"
        );
    }
}
