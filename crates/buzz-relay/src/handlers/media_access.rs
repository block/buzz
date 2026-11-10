//! Prevent a copied inaccessible URL from granting access through a new event.
use super::ingest::IngestError;
use crate::{api::media_access::MediaReadMode, state::AppState};
use buzz_core::TenantContext;
use nostr::Event;

pub(crate) async fn validate_media_references(
    tenant: &TenantContext,
    state: &AppState,
    event: &Event,
) -> Result<(), IngestError> {
    let mode = state.config.media_read_mode;
    if mode == MediaReadMode::Off {
        return Ok(());
    }
    let base = crate::api::media::media_base_url_for_tenant(&state.config.relay_url, tenant.host());
    let hashes = buzz_core::media::publication_hashes(event, &base);
    // The bounded profile read probe searches the original signed JSON for the
    // literal hash. Do not accept an encoded-only reference that probe cannot find.
    if !profile_hashes_are_literal(event, &hashes) {
        metrics::counter!("buzz_media_publication_access_total", "mode" => mode.label(), "reason" => "noncanonical_profile_hash").increment(1);
        if mode == MediaReadMode::Enforce {
            return Err(IngestError::Rejected(
                "restricted: profile media hashes must appear literally in profile content".into(),
            ));
        }
    }
    if hashes.len() > 512 {
        metrics::counter!("buzz_media_publication_access_total", "mode" => mode.label(), "reason" => "limit").increment(1);
        return if mode == MediaReadMode::Enforce {
            Err(IngestError::Rejected(
                "restricted: too many attachment references".into(),
            ))
        } else {
            // Skip per-reference fan-out, but retain a single best-effort
            // publication boundary batch for accepted shadow events.
            if state
                .db
                .record_media_publications(tenant.community(), hashes.into_iter().collect())
                .await
                .is_err()
            {
                metrics::counter!("buzz_media_publication_access_total", "mode" => mode.label(), "reason" => "record_error").increment(1);
            }
            Ok(())
        };
    }
    let author = super::ingest::effective_message_author(event, &state.relay_keypair.public_key());
    for hash in &hashes {
        let checked = async {
            let meta = state
                .media_storage
                .get_sidecar(tenant, hash)
                .await
                .map_err(|error| {
                    if matches!(error, buzz_media::MediaError::NotFound) {
                        (
                            "missing_sidecar",
                            IngestError::Rejected(
                                "restricted: attachment is not accessible".into(),
                            ),
                        )
                    } else {
                        (
                            "lookup_error",
                            IngestError::Internal("error: media reference lookup failed".into()),
                        )
                    }
                })?;
            let access = state
                .db
                .media_access(
                    tenant.community(),
                    hash,
                    &meta.ext,
                    &base,
                    &author,
                    true,
                    super::req::event_visible_to_reader,
                )
                .await
                .map_err(|error| {
                    (
                        if matches!(error, buzz_db::DbError::MediaAuthorizationCapacity) {
                            "capacity"
                        } else {
                            "lookup_error"
                        },
                        IngestError::Internal("error: media authorization unavailable".into()),
                    )
                })?;
            if matches!(
                access,
                buzz_db::media::MediaAccess::Allowed | buzz_db::media::MediaAccess::UploaderPreview
            ) {
                Ok(())
            } else {
                Err((
                    "denied",
                    IngestError::Rejected("restricted: attachment is not accessible".into()),
                ))
            }
        }
        .await;
        let reason = checked
            .as_ref()
            .err()
            .map_or("allow", |(reason, _)| *reason);
        metrics::counter!("buzz_media_publication_access_total", "mode" => mode.label(), "reason" => reason).increment(1);
        if let Err((_, error)) = checked {
            if mode == MediaReadMode::Enforce {
                return Err(error);
            }
        }
    }
    // A marker consumes previews but never grants download access. Shadow
    // recording is best effort and cannot change event acceptance on failure.
    if !hashes.is_empty()
        && state
            .db
            .record_media_publications(tenant.community(), hashes.into_iter().collect())
            .await
            .is_err()
    {
        metrics::counter!("buzz_media_publication_access_total", "mode" => mode.label(), "reason" => "record_error").increment(1);
        if mode == MediaReadMode::Enforce {
            return Err(IngestError::Internal(
                "error: media publication recording failed".into(),
            ));
        }
    }
    Ok(())
}

fn profile_hashes_are_literal(event: &Event, hashes: &std::collections::BTreeSet<String>) -> bool {
    !matches!(event.kind.as_u16(), 0 | 30175)
        || hashes.iter().all(|hash| event.content.contains(hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_publication_matches_the_bounded_literal_read_probe() {
        let keys = nostr::Keys::generate();
        let hash = "f".repeat(64);
        let poster = "a".repeat(64);
        let base = "https://example.org/media";
        let plain = format!(r#"{{"picture":"{base}/{hash}.png"}}"#);
        let escaped = plain.replace(&hash, &format!("\\u0066{}", &hash[1..]));
        let animated = format!(
            r#"{{"picture":"{base}/{poster}.png#buzz-anim=https%3A%2F%2Fexample.org%2Fmedia%2F%66{}.gif"}}"#,
            &hash[1..]
        );
        for (content, accepted) in [(plain, true), (escaped, false), (animated, false)] {
            let event = nostr::EventBuilder::new(nostr::Kind::Metadata, content)
                .sign_with_keys(&keys)
                .unwrap();
            let hashes = buzz_core::media::publication_hashes(&event, base);
            assert!(hashes.contains(&hash));
            assert_eq!(profile_hashes_are_literal(&event, &hashes), accepted);
        }
    }
}
