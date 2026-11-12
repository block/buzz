//! Explicit Object Lock reads. HEAD omits lock headers without the corresponding
//! permissions, so missing HEAD headers cannot establish that deletion is allowed.
use super::*;
use chrono::{DateTime, Utc};

/// Storage restrictions for one immutable object version.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionRetention {
    /// An indefinite legal hold, independent of the retention date.
    pub legal_hold: bool,
    /// Expiry supplied by storage; governance retention is never bypassed.
    pub retain_until: Option<DateTime<Utc>>,
}

impl VersionRetention {
    /// Whether deleting this version is prohibited at the supplied instant.
    pub fn prevents_deletion(&self, now: DateTime<Utc>) -> bool {
        self.legal_hold || self.retain_until.is_some_and(|until| until > now)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegalHold {
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Retention {
    mode: Option<String>,
    retain_until_date: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct StorageError {
    code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LockConfiguration {
    object_lock_enabled: String,
}

impl MediaStorage {
    /// Read legal hold and retention separately for the exact version. Permission
    /// failures, missing versions and unsupported APIs remain errors. Only S3's
    /// explicit ObjectLockConfigurationNotFoundError establishes no Object Lock.
    pub async fn version_retention(
        &self,
        key: &str,
        version_id: &str,
    ) -> Result<VersionRetention, MediaError> {
        if version_id.is_empty() || version_id == "null" {
            return Err(xml_error("retention check requires an immutable version"));
        }
        let config = self.lock_document("/", None, "object-lock").await?;
        let Some(config) = config else {
            return Ok(VersionRetention {
                legal_hold: false,
                retain_until: None,
            });
        };
        let config: LockConfiguration =
            quick_xml::de::from_reader(config.as_slice()).map_err(xml_error)?;
        if config.object_lock_enabled != "Enabled" {
            return Err(xml_error("invalid Object Lock configuration"));
        }
        let hold = self
            .lock_document(key, Some(version_id), "legal-hold")
            .await?;
        let retention = self
            .lock_document(key, Some(version_id), "retention")
            .await?;
        parse_restrictions(hold.as_deref(), retention.as_deref())
    }

    async fn lock_document(
        &self,
        key: &str,
        version_id: Option<&str>,
        operation: &str,
    ) -> Result<Option<Vec<u8>>, MediaError> {
        let mut query = HashMap::from([(operation.to_owned(), String::new())]);
        if let Some(version_id) = version_id {
            query.insert("versionId".to_owned(), version_id.to_owned());
        }
        let bucket = self.bucket.with_extra_query(query).map_err(xml_error)?;
        let request = s3::request::tokio_backend::ReqwestRequest::new(
            &bucket,
            key,
            s3::command::Command::GetObject,
        )
        .await
        .map_err(xml_error)?;
        let response = match request.response().await {
            Ok(response) => response,
            Err(s3::error::S3Error::HttpFailWithBody(404, body)) => {
                if operation == "object-lock"
                    && body.len() <= 65_536
                    && quick_xml::de::from_str::<StorageError>(&body)
                        .is_ok_and(|e| e.code == "ObjectLockConfigurationNotFoundError")
                {
                    return Ok(None);
                }
                return Err(xml_error("Object Lock read returned an unexplained 404"));
            }
            Err(error) => return Err(xml_error(error)),
        };
        if !response.status().is_success() {
            return Err(xml_error(format!(
                "Object Lock read failed: {}",
                response.status()
            )));
        }
        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(xml_error)?;
            if bytes.len() + chunk.len() > 65_536 {
                return Err(xml_error("Object Lock response exceeds 64 KiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }
}

fn parse_restrictions(
    hold: Option<&[u8]>,
    retention: Option<&[u8]>,
) -> Result<VersionRetention, MediaError> {
    let legal_hold = match hold {
        None => false,
        Some(xml) => match quick_xml::de::from_reader::<_, LegalHold>(xml)
            .map_err(xml_error)?
            .status
            .as_str()
        {
            "ON" => true,
            "OFF" => false,
            _ => return Err(xml_error("invalid legal hold status")),
        },
    };
    let retain_until = match retention {
        None => None,
        Some(xml) => {
            let value: Retention = quick_xml::de::from_reader(xml).map_err(xml_error)?;
            match (value.mode.as_deref(), value.retain_until_date) {
                (None, None) => None,
                (Some("GOVERNANCE" | "COMPLIANCE"), Some(until)) => Some(until),
                _ => return Err(xml_error("invalid retention mode/date")),
            }
        }
    };
    Ok(VersionRetention {
        legal_hold,
        retain_until,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hold_and_both_retention_modes_prevent_deletion_without_bypass() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for mode in ["GOVERNANCE", "COMPLIANCE"] {
            let xml = format!("<Retention><Mode>{mode}</Mode><RetainUntilDate>2027-01-01T00:00:00Z</RetainUntilDate></Retention>");
            let restriction = parse_restrictions(
                Some(b"<LegalHold><Status>OFF</Status></LegalHold>"),
                Some(xml.as_bytes()),
            )
            .unwrap();
            assert!(restriction.prevents_deletion(now));
            assert!(!restriction.prevents_deletion(now + chrono::Duration::days(800)));
        }
        assert!(
            parse_restrictions(Some(b"<LegalHold><Status>ON</Status></LegalHold>"), None)
                .unwrap()
                .prevents_deletion(now)
        );
        assert!(!parse_restrictions(None, None)
            .unwrap()
            .prevents_deletion(now));
        assert!(parse_restrictions(Some(b"<LegalHold/>"), None).is_err());
        assert!(parse_restrictions(
            None,
            Some(b"<Retention><Mode>COMPLIANCE</Mode></Retention>")
        )
        .is_err());
    }
}
