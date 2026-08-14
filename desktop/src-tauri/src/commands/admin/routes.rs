//! Closed route enum and typed query parameters for the admin API.
//!
//! No IPC surface accepts an arbitrary path; every URL is constructed here
//! from a typed route and typed query parameters. IDs are carried as `Uuid`
//! values so path injection is structurally impossible; the attachment hash is
//! validated to match the relay's exact lowercase-hex-only grammar before a
//! route is constructed.

/// A validated lowercase 64-hex SHA-256 hash suitable for use as an attachment
/// path segment. Constructed only through [`AttachmentHash::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentHash(String);

impl AttachmentHash {
    /// Parse `raw` as a lowercase 64-hex SHA-256. Returns `Err` for any input
    /// that isn't exactly 64 lowercase hex digits, including uppercase A-F (the
    /// relay stores lowercase and returns 404 on uppercase).
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() != 64 {
            return Err(format!(
                "attachment hash must be exactly 64 hex characters; got {} characters",
                raw.len()
            ));
        }
        if !raw.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            return Err("attachment hash must be lowercase hex only (0-9, a-f); \
                 uppercase is rejected — the relay stores lowercase and returns 404 otherwise"
                .to_string());
        }
        Ok(AttachmentHash(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The routes exposed by `/api/admin/v1`.
///
/// IDs are typed `Uuid` — path injection via slash, `..`, `?`, `#`, or
/// percent-escapes is structurally impossible. The attachment hash is an
/// `AttachmentHash`, enforcing exact lowercase-hex grammar. Operator pubkeys
/// are validated hex strings.
#[derive(Debug)]
pub enum AdminRoute {
    /// Auth-mode/role/capability discovery. Requires no DB and returns role
    /// `null` in token/disabled modes.
    Probe,
    ReportsList,
    ReportDetail {
        id: uuid::Uuid,
    },
    ReportResolve {
        id: uuid::Uuid,
    },
    ReportReopen {
        id: uuid::Uuid,
    },
    ReportCancel {
        id: uuid::Uuid,
    },
    FeedbackList,
    FeedbackDetail {
        id: uuid::Uuid,
    },
    FeedbackAttachment {
        id: uuid::Uuid,
        sha256: AttachmentHash,
    },
    FeedbackPatch {
        id: uuid::Uuid,
    },
    OperatorsList,
    OperatorPut {
        pubkey: HexPubkey,
    },
    OperatorDelete {
        pubkey: HexPubkey,
    },
}

/// A validated 64 lowercase-hex character pubkey for use as a URL path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexPubkey(String);

impl HexPubkey {
    /// Parse `raw` as a 64-character lowercase hex pubkey.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() != 64 {
            return Err(format!(
                "pubkey must be exactly 64 hex characters; got {}",
                raw.len()
            ));
        }
        if !raw.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')) {
            return Err("pubkey must be lowercase hex only (0-9, a-f)".to_string());
        }
        Ok(HexPubkey(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AdminRoute {
    /// Return the URL path component (not including the `/api/admin/v1` prefix).
    pub fn path(&self) -> String {
        match self {
            AdminRoute::Probe => "/probe".to_string(),
            AdminRoute::ReportsList => "/reports".to_string(),
            AdminRoute::ReportDetail { id } => format!("/reports/{id}"),
            AdminRoute::ReportResolve { id } => format!("/reports/{id}/resolve"),
            AdminRoute::ReportReopen { id } => format!("/reports/{id}/reopen"),
            AdminRoute::ReportCancel { id } => format!("/reports/{id}/cancel"),
            AdminRoute::FeedbackList => "/feedback".to_string(),
            AdminRoute::FeedbackDetail { id } => format!("/feedback/{id}"),
            AdminRoute::FeedbackAttachment { id, sha256 } => {
                format!("/feedback/{id}/attachments/{}", sha256.as_str())
            }
            AdminRoute::FeedbackPatch { id } => format!("/feedback/{id}"),
            AdminRoute::OperatorsList => "/operators".to_string(),
            AdminRoute::OperatorPut { pubkey } => format!("/operators/{}", pubkey.as_str()),
            AdminRoute::OperatorDelete { pubkey } => format!("/operators/{}", pubkey.as_str()),
        }
    }
}

/// Optional query parameters for the reports-list endpoint.
///
/// All fields are `Option<String>` so the struct can be constructed with only
/// the fields the caller cares about; `to_query_string` omits `None` fields.
#[derive(Debug, Default)]
pub struct AdminQuery {
    pub community_id: Option<String>,
    pub status: Option<String>,
    pub report_type: Option<String>,
    pub target_kind: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: Option<i64>,
}

impl AdminQuery {
    /// Serialise to a URL query string (no leading `?`). Returns an empty
    /// string when all fields are `None`.
    pub fn to_query_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = &self.community_id {
            parts.push(format!("communityId={}", urlencoded(v)));
        }
        if let Some(v) = &self.status {
            parts.push(format!("status={}", urlencoded(v)));
        }
        if let Some(v) = &self.report_type {
            parts.push(format!("reportType={}", urlencoded(v)));
        }
        if let Some(v) = &self.target_kind {
            parts.push(format!("targetKind={}", urlencoded(v)));
        }
        if let Some(v) = &self.after {
            parts.push(format!("after={}", urlencoded(v)));
        }
        if let Some(v) = &self.before {
            parts.push(format!("before={}", urlencoded(v)));
        }
        if let Some(v) = &self.limit {
            parts.push(format!("limit={v}"));
        }
        parts.join("&")
    }
}

/// Percent-encode a query parameter value, matching `url::form_urlencoded`.
fn urlencoded(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AttachmentHash validation ─────────────────────────────────────────────

    #[test]
    fn attachment_hash_valid_lowercase_hex() {
        let h = AttachmentHash::parse(&"a".repeat(64)).unwrap();
        assert_eq!(h.as_str(), "a".repeat(64));
    }

    #[test]
    fn attachment_hash_rejects_too_short() {
        assert!(AttachmentHash::parse(&"a".repeat(63)).is_err());
    }

    #[test]
    fn attachment_hash_rejects_too_long() {
        assert!(AttachmentHash::parse(&"a".repeat(65)).is_err());
    }

    #[test]
    fn attachment_hash_rejects_uppercase() {
        // Uppercase passes is_ascii_hexdigit() but the relay returns 404 for it.
        // AttachmentHash::parse must reject uppercase.
        assert!(AttachmentHash::parse(&"A".repeat(64)).is_err());
        let mixed = format!("{}A{}", "a".repeat(32), "a".repeat(31));
        assert!(AttachmentHash::parse(&mixed).is_err());
    }

    #[test]
    fn attachment_hash_rejects_non_hex_chars() {
        // 'g' is not a hex digit.
        assert!(AttachmentHash::parse(&"g".repeat(64)).is_err());
    }

    #[test]
    fn attachment_hash_rejects_slash() {
        let s = format!("{}/{}", "a".repeat(32), "a".repeat(31));
        assert!(AttachmentHash::parse(&s).is_err());
    }

    #[test]
    fn attachment_hash_rejects_dot_dot() {
        let s = format!("{}..{}", "a".repeat(31), "a".repeat(31));
        assert!(AttachmentHash::parse(&s).is_err());
    }

    #[test]
    fn attachment_hash_rejects_percent_escape() {
        // URL-encoded slash would be %2F — 3 chars, must fail length check too.
        assert!(AttachmentHash::parse("%2F").is_err());
        // But also reject any % in a 64-char input.
        let s = format!("{}%2{}", "a".repeat(31), "a".repeat(31));
        assert!(AttachmentHash::parse(&s).is_err());
    }

    #[test]
    fn attachment_hash_rejects_query_fragment() {
        let s = format!("{}?{}", "a".repeat(32), "a".repeat(31));
        assert!(AttachmentHash::parse(&s).is_err());
        let s2 = format!("{}#{}", "a".repeat(32), "a".repeat(31));
        assert!(AttachmentHash::parse(&s2).is_err());
    }

    // ── AdminRoute::path ─────────────────────────────────────────────────────

    #[test]
    fn reports_list_path() {
        assert_eq!(AdminRoute::ReportsList.path(), "/reports");
    }

    #[test]
    fn probe_path() {
        assert_eq!(AdminRoute::Probe.path(), "/probe");
    }

    #[test]
    fn report_detail_path() {
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        assert_eq!(
            AdminRoute::ReportDetail { id }.path(),
            "/reports/00000000-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn report_reopen_path() {
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
        assert_eq!(
            AdminRoute::ReportReopen { id }.path(),
            "/reports/00000000-0000-0000-0000-000000000003/reopen"
        );
    }

    #[test]
    fn report_cancel_path() {
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        assert_eq!(
            AdminRoute::ReportCancel { id }.path(),
            "/reports/00000000-0000-0000-0000-000000000004/cancel"
        );
    }

    #[test]
    fn feedback_attachment_path() {
        let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let hash = AttachmentHash::parse(&"ab".repeat(32)).unwrap();
        let path = AdminRoute::FeedbackAttachment {
            id,
            sha256: hash.clone(),
        }
        .path();
        assert_eq!(
            path,
            format!(
                "/feedback/00000000-0000-0000-0000-000000000002/attachments/{}",
                hash.as_str()
            )
        );
    }

    // ── AdminQuery ───────────────────────────────────────────────────────────

    #[test]
    fn query_empty_produces_no_string() {
        assert_eq!(AdminQuery::default().to_query_string(), "");
    }

    #[test]
    fn query_limit_only() {
        let q = AdminQuery {
            limit: Some(50),
            ..Default::default()
        };
        assert_eq!(q.to_query_string(), "limit=50");
    }

    #[test]
    fn query_multiple_params() {
        let q = AdminQuery {
            status: Some("open".to_string()),
            limit: Some(100),
            ..Default::default()
        };
        let qs = q.to_query_string();
        assert!(qs.contains("status=open"), "expected status in {qs}");
        assert!(qs.contains("limit=100"), "expected limit in {qs}");
    }

    #[test]
    fn query_value_is_percent_encoded() {
        let q = AdminQuery {
            status: Some("open&active".to_string()),
            ..Default::default()
        };
        let qs = q.to_query_string();
        // & in value must be encoded so it doesn't split the query.
        assert!(!qs.contains("status=open&active"), "bare & leaked: {qs}");
        assert!(qs.contains("status="), "status key missing: {qs}");
    }
}
