//! Complete structural relay matching over data-only input records.
use super::input::{EventInput, EventTag, FilterInput, TagQuery};

#[inline]
fn bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let n = left.len();
    if n == 1 {
        return left[0] == right[0];
    }
    if n > 0 && left[n - 1] != right[n - 1] {
        return false;
    }
    if n >= 4 {
        let i = n - 4;
        if !((left[i] == right[i]) & (left[i + 1] == right[i + 1]) & (left[i + 2] == right[i + 2]))
        {
            return false;
        }
    }
    left == right
}

fn contains_id(values: &[[u8; 32]], id: &[u8; 32]) -> bool {
    let mut i = values.len();
    while i > 0 {
        i -= 1;
        if bytes_equal(&values[i], id) {
            return true;
        }
    }
    false
}

fn contains_kind(values: &[u16], kind: u16) -> bool {
    let mut i = values.len();
    while i > 0 {
        i -= 1;
        if values[i] == kind {
            return true;
        }
    }
    false
}

fn contains_value(values: &[Vec<u8>], value: &[u8]) -> bool {
    let mut i = values.len();
    if i == 0 {
        return false;
    }
    if bytes_equal(&values[0], value) {
        return true;
    }
    while i > 1 {
        i -= 1;
        if bytes_equal(&values[i], value) {
            return true;
        }
    }
    false
}

fn is_h(key: &[u8]) -> bool {
    key.len() == 1 && key[0] == b"h"[0]
}

fn tag_matches(query: &TagQuery, tag: &EventTag) -> bool {
    if bytes_equal(&tag.key, &query.key) {
        if let Some(value) = &tag.value {
            return contains_value(&query.values, value);
        }
    }
    false
}

fn explicit_match(query: &TagQuery, event: &EventInput) -> bool {
    let mut i = event.tags.len();
    if i == 0 {
        return false;
    }
    if tag_matches(query, &event.tags[0]) {
        return true;
    }
    while i > 1 {
        i -= 1;
        if tag_matches(query, &event.tags[i]) {
            return true;
        }
    }
    false
}

fn has_h(event: &EventInput) -> bool {
    let mut i = event.tags.len();
    while i > 0 {
        i -= 1;
        if is_h(&event.tags[i].key) {
            return true;
        }
    }
    false
}

fn tag_constraint(query: &TagQuery, event: &EventInput) -> bool {
    if explicit_match(query, event) {
        return true;
    }
    if is_h(&query.key) && !has_h(event) {
        if let Some(channel) = &event.channel {
            return contains_value(&query.values, channel);
        }
    }
    false
}

fn tags_match(filter: &FilterInput, event: &EventInput) -> bool {
    let mut i = filter.tags.len();
    while i > 0 {
        i -= 1;
        if !tag_constraint(&filter.tags[i], event) {
            return false;
        }
    }
    true
}

fn one_filter(filter: &FilterInput, event: &EventInput) -> bool {
    if let Some(ids) = &filter.ids {
        if !contains_id(ids, &event.id) {
            return false;
        }
    }
    if let Some(authors) = &filter.authors {
        if !contains_id(authors, &event.author) {
            return false;
        }
    }
    if let Some(kinds) = &filter.kinds {
        if !contains_kind(kinds, event.kind) {
            return false;
        }
    }
    if let Some(since) = filter.since {
        if event.created_at < since {
            return false;
        }
    }
    if let Some(until) = filter.until {
        if until < event.created_at {
            return false;
        }
    }
    tags_match(filter, event)
}

/// Match an event against the OR of complete NIP-01 filters.
pub fn filters_match(filters: &[FilterInput], event: &EventInput) -> bool {
    let mut i = filters.len();
    while i > 0 {
        i -= 1;
        if one_filter(&filters[i], event) {
            return true;
        }
    }
    false
}
