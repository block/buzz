//! Exact historical matcher from base 71d7aa04369a6e86e158dc6d46246c8a3ce6d166. Benchmark-only.
use buzz_core::StoredEvent;
use nostr::Filter;
pub fn filters_match(filters: &[Filter], event: &StoredEvent) -> bool {
    filters.iter().any(|f| filter_match_one(f, event))
}

fn filter_match_one(f: &Filter, ev: &StoredEvent) -> bool {
    if let Some(kinds) = &f.kinds {
        if !kinds.contains(&ev.event.kind) {
            return false;
        }
    }

    if let Some(authors) = &f.authors {
        if !authors.contains(&ev.event.pubkey) {
            return false;
        }
    }

    if let Some(since) = f.since {
        if ev.event.created_at < since {
            return false;
        }
    }

    if let Some(until) = f.until {
        if ev.event.created_at > until {
            return false;
        }
    }

    // NIP-01 allows prefix matching on event IDs.
    if let Some(ids) = &f.ids {
        let event_id_hex = ev.event.id.to_hex();
        if !ids.iter().any(|id| event_id_hex.starts_with(&id.to_hex())) {
            return false;
        }
    }

    for (tag_key, tag_values) in f.generic_tags.iter() {
        let tag_key_str = tag_key.to_string();
        let has_match = tag_values.iter().any(|filter_val| {
            ev.event
                .tags
                .iter()
                .filter(|t| t.kind().to_string() == tag_key_str)
                .filter_map(|t| t.content())
                .any(|event_val| event_val == filter_val.as_str())
        });
        // Fallback for #h (channel) filters: some events (reactions kind:7,
        // deletions kind:5) derive their channel from the target event and
        // don't carry an h-tag themselves. Use StoredEvent.channel_id as a
        // fallback ONLY when the event has no h-tags at all — if the event
        // has explicit h-tags, those are authoritative and must match.
        if !has_match && tag_key_str == "h" {
            let event_has_h_tags = ev.event.tags.iter().any(|t| t.kind().to_string() == "h");
            if !event_has_h_tags {
                if let Some(ch_id) = ev.channel_id {
                    let ch_str = ch_id.to_string();
                    if !tag_values.iter().any(|v| v.as_str() == ch_str) {
                        return false;
                    }
                } else {
                    return false;
                }
            } else {
                // Event has h-tags but none matched — strict rejection.
                return false;
            }
        } else if !has_match {
            return false;
        }
    }

    true
}
