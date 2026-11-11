//! Channel media policy, separate from Blossom proof/community admission.
use super::media::MediaDenial;
use crate::state::AppState;
use buzz_core::TenantContext;
use buzz_media::{auth::BlossomStrictness, MediaError};

/// Startup-selected rollout policy. Off preserves the current community gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaReadMode {
    /// Skip reference authorization (does not provide channel revocation).
    #[default]
    Off,
    /// Evaluate and count the policy without refusing reads/publications.
    Shadow,
    /// Enforce fresh channel and event authorization.
    Enforce,
}
impl std::str::FromStr for MediaReadMode {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "shadow" => Ok(Self::Shadow),
            "enforce" => Ok(Self::Enforce),
            _ => Err("BUZZ_MEDIA_READ_MODE must be off, shadow, or enforce".into()),
        }
    }
}
impl MediaReadMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Shadow => "shadow",
            Self::Enforce => "enforce",
        }
    }
}

/// Authorize the parent hash before any blob/thumbnail/range information.
/// The tenant sidecar is a cheap first gate before the new DB lookup.
pub(super) async fn enforce_read(
    state: &AppState,
    tenant: &TenantContext,
    path: &str,
    pubkey: &[u8],
    strictness: BlossomStrictness,
) -> Result<(), MediaDenial> {
    let mode = state.config.media_read_mode;
    if mode == MediaReadMode::Off {
        return Ok(());
    }
    let hash = path
        .split('.')
        .next()
        .ok_or(MediaDenial(MediaError::NotFound, strictness))?;
    let meta = match state.media_storage.get_sidecar(tenant, hash).await {
        Ok(meta) => meta,
        Err(error) => {
            let missing = matches!(&error, MediaError::NotFound);
            metrics::counter!("buzz_media_read_access_total","mode"=>mode.label(),"reason"=>if missing {"no_ref"} else {"lookup_error"}).increment(1);
            if mode == MediaReadMode::Shadow {
                return Ok(());
            }
            return Err(MediaDenial(
                if missing {
                    MediaError::NotFound
                } else {
                    MediaError::ServiceUnavailable
                },
                strictness,
            ));
        }
    };
    let base = super::media::media_base_url_for_tenant(&state.config.relay_url, tenant.host());
    let result = state
        .db
        .media_access(
            tenant.community(),
            hash,
            &meta.ext,
            &base,
            pubkey,
            false,
            crate::handlers::req::event_visible_to_reader,
        )
        .await;
    let (reason, error) = match result {
        Ok(buzz_db::media::MediaAccess::Allowed | buzz_db::media::MediaAccess::UploaderPreview) => {
            ("allow", None)
        }
        Ok(buzz_db::media::MediaAccess::NoReference) => ("no_ref", Some(MediaError::NotFound)),
        Ok(buzz_db::media::MediaAccess::Restricted) => ("not_member", Some(MediaError::NotFound)),
        Err(error) => {
            tracing::warn!(error_kind = %error.kind(), "media authorization lookup failed");
            (
                if matches!(error, buzz_db::DbError::MediaAuthorizationCapacity) {
                    "capacity"
                } else {
                    "lookup_error"
                },
                Some(MediaError::AuthorizationUnavailable),
            )
        }
    };
    metrics::counter!("buzz_media_read_access_total", "mode" => mode.label(), "reason" => reason)
        .increment(1);
    if mode == MediaReadMode::Enforce {
        if let Some(error) = error {
            return Err(MediaDenial(error, strictness));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rollout_mode_is_explicit() {
        for mode in ["off", "shadow", "enforce"] {
            assert!(mode.parse::<MediaReadMode>().is_ok());
        }
        assert!("typo".parse::<MediaReadMode>().is_err());
        assert_eq!(MediaReadMode::default(), MediaReadMode::Off);
    }
}
