//! COMPLETE structural NIP-01 matcher: declarative vocabulary, no search algorithm.
//! None constraints are unrestricted; Some(empty) reject. Exact bytes, no normalization.
//! No implementation preconditions. Search/limit belong to query execution, not this predicate.
use crate::input::{EventInput, EventTag, FilterInput, TagQuery};
use djinn::djinn;

djinn! {
    #[total]
    pub rel identities(ids: Option<Vec<[u8; 32]>>, id: &[u8; 32]) -> r: bool {
        if ids.is_none() { r == true; }
        else { r == ids.unwrap().iter().any(|value| value == id); }
    }

    #[total]
    pub rel kinds(kinds: Option<Vec<u16>>, kind: u16) -> r: bool {
        if kinds.is_none() { r == true; }
        else { r == kinds.unwrap().iter().any(|value| value == kind); }
    }

    #[total]
    pub rel since(bound: Option<u64>, time: u64) -> r: bool {
        if bound.is_none() { r == true; }
        else { r == (bound.unwrap() <= time); }
    }

    #[total]
    pub rel until(bound: Option<u64>, time: u64) -> r: bool {
        if bound.is_none() { r == true; }
        else { r == (time <= bound.unwrap()); }
    }

    // Only the second string is a value; None differs from an empty byte string.
    #[total]
    pub rel tag_match(tag: &EventTag, key: &[u8], values: &[Vec<u8>]) -> r: bool {
        if tag.value.is_none() { r == false; }
        else {
            some value: Vec<u8> {
                value == tag.value.unwrap();
                r == (tag.key == key && values.iter().any(|v| v == value));
            }
        }
    }

    // An explicit lower-case h tag, even contentless, disables stored UUID fallback.
    #[total]
    pub rel tag_constraint(query: &TagQuery, event: &EventInput) -> r: bool {
        some explicit: bool {
            if explicit {
                some tag: EventTag {
                    tag in event.tags;
                    tag_match(tag, query.key, query.values) == true;
                }
            } else {
                for tag in event.tags { tag_match(tag, query.key, query.values) == false; }
            }
            if event.channel.is_none() { r == explicit; }
            else {
                some channel: Vec<u8> {
                    channel == event.channel.unwrap();
                    r == (explicit || (query.key == b"h"
                        && !event.tags.iter().any(|tag| tag.key == b"h")
                        && query.values.iter().any(|value| value == channel)));
                }
            }
        }
    }

    // All fields and all tag keys must hold for this SAME filter.
    #[total]
    pub rel one_filter(filter: &FilterInput, event: &EventInput) -> r: bool {
        some tags_ok: bool {
            if tags_ok {
                for query in filter.tags { tag_constraint(query, event) == true; }
            } else {
                some query: TagQuery {
                    query in filter.tags;
                    tag_constraint(query, event) == false;
                }
            }
            r == (identities(filter.ids, event.id)
                && identities(filter.authors, event.author)
                && kinds(filter.kinds, event.kind)
                && since(filter.since, event.created_at)
                && until(filter.until, event.created_at)
                && tags_ok);
        }
    }

    // True iff at least one WHOLE filter holds; false iff every whole filter fails.
    pub fn filters_match(filters: &[FilterInput], event: &EventInput) -> r: bool {
        some matched: bool {
            if matched {
                some filter: FilterInput {
                    filter in filters;
                    one_filter(filter, event) == true;
                }
            } else {
                for filter in filters { one_filter(filter, event) == false; }
            }
            r == matched;
        }
    }
}
