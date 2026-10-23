//! Public-seam contract matrix. Expected answers are explicit, not computed by a matcher.
use super::filters_match;
use crate::StoredEvent;
use chrono::{TimeZone, Utc};
use nostr::{Event, Filter};
use serde_json::{json, Value};
use uuid::Uuid;

const AUTHOR: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
const OTHER_AUTHOR: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

#[test]
fn structural_contract_matrix() {
    let channel = Uuid::from_u128(0xa0112233445546778899aabbccdd0042);
    let h = channel.to_string();
    let id = "01".repeat(32);
    let mut last_byte_id = id.clone();
    last_byte_id.replace_range(62..64, "02");
    let mut last_byte_author = AUTHOR.to_owned();
    last_byte_author.replace_range(62..64, "99");
    // Every row varies the complete public input, including missing/empty tag content.
    type ContractCase<'a> = (
        &'a str,
        Vec<Value>,
        Vec<Vec<&'a str>>,
        Option<Uuid>,
        u16,
        u64,
        bool,
    );
    let cases: Vec<ContractCase<'_>> = vec![
        ("no filters", vec![], vec![], None, 1, 123, false),
        ("empty filter", vec![json!({})], vec![], None, 1, 123, true),
        (
            "empty ids",
            vec![json!({"ids":[]})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "empty authors",
            vec![json!({"authors":[]})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "empty kinds",
            vec![json!({"kinds":[]})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "exact id",
            vec![json!({"ids":[id]})],
            vec![],
            None,
            1,
            123,
            true,
        ),
        (
            "last id byte differs",
            vec![json!({"ids":[last_byte_id]})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "exact author",
            vec![json!({"authors":[AUTHOR]})],
            vec![],
            None,
            1,
            123,
            true,
        ),
        (
            "last author byte differs",
            vec![json!({"authors":[last_byte_author]})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "inclusive times",
            vec![json!({"since":123,"until":123})],
            vec![],
            None,
            1,
            123,
            true,
        ),
        (
            "since excludes",
            vec![json!({"since":124})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "until excludes",
            vec![json!({"until":122})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "contradictory times",
            vec![json!({"since":124,"until":122})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "unsigned max time",
            vec![json!({"since":u64::MAX,"until":u64::MAX})],
            vec![],
            None,
            65535,
            u64::MAX,
            true,
        ),
        (
            "kind width",
            vec![json!({"kinds":[65535]})],
            vec![],
            None,
            65535,
            123,
            true,
        ),
        (
            "OR late filter",
            vec![json!({"kinds":[9]}), json!({"kinds":[1]})],
            vec![],
            None,
            1,
            123,
            true,
        ),
        (
            "no cross-filter union",
            vec![
                json!({"kinds":[9],"authors":[OTHER_AUTHOR]}),
                json!({"kinds":[1],"authors":[AUTHOR]}),
            ],
            vec![],
            None,
            9,
            123,
            false,
        ),
        (
            "AND tag keys",
            vec![json!({"#p":["a"],"#e":["a"]})],
            vec![vec!["p", "a"]],
            None,
            1,
            123,
            false,
        ),
        (
            "both keys hit",
            vec![json!({"#p":["a"],"#e":["b"]})],
            vec![vec!["p", "a"], vec!["e", "b"]],
            None,
            1,
            123,
            true,
        ),
        (
            "OR tag values",
            vec![json!({"#p":["a","b"]})],
            vec![vec!["p", "b"]],
            None,
            1,
            123,
            true,
        ),
        (
            "later tag fields ignored",
            vec![json!({"#p":["b"]})],
            vec![vec!["p", "a", "b"]],
            None,
            1,
            123,
            false,
        ),
        (
            "late duplicate hit",
            vec![json!({"#p":["a"]})],
            vec![vec!["p", "other"], vec!["p", "a"]],
            None,
            1,
            123,
            true,
        ),
        (
            "empty value matches",
            vec![json!({"#p":[""]})],
            vec![vec!["p", ""]],
            None,
            1,
            123,
            true,
        ),
        (
            "missing value differs",
            vec![json!({"#p":[""]})],
            vec![vec!["p"]],
            None,
            1,
            123,
            false,
        ),
        (
            "Unicode exact",
            vec![json!({"#p":["é🙂"]})],
            vec![vec!["p", "é🙂"]],
            None,
            1,
            123,
            true,
        ),
        (
            "no Unicode normalization",
            vec![json!({"#p":["é"]})],
            vec![vec!["p", "e\u{301}"]],
            None,
            1,
            123,
            false,
        ),
        (
            "case-sensitive keys",
            vec![json!({"#p":["a"]})],
            vec![vec!["P", "a"]],
            None,
            1,
            123,
            false,
        ),
        (
            "case-sensitive values",
            vec![json!({"#p":["A"]})],
            vec![vec!["p", "a"]],
            None,
            1,
            123,
            false,
        ),
        (
            "non-letter event keys",
            vec![json!({"#p":["a"]})],
            vec![vec![""], vec!["é", "a"]],
            None,
            1,
            123,
            false,
        ),
        (
            "h fallback",
            vec![json!({"#h":[h]})],
            vec![],
            Some(channel),
            1,
            123,
            true,
        ),
        (
            "no stored channel",
            vec![json!({"#h":[h]})],
            vec![],
            None,
            1,
            123,
            false,
        ),
        (
            "explicit h defeats fallback",
            vec![json!({"#h":[h]})],
            vec![vec!["h", "other"]],
            Some(channel),
            1,
            123,
            false,
        ),
        (
            "contentless h defeats fallback",
            vec![json!({"#h":[h]})],
            vec![vec!["h"]],
            Some(channel),
            1,
            123,
            false,
        ),
        (
            "empty h defeats fallback",
            vec![json!({"#h":[h]})],
            vec![vec!["h", ""]],
            Some(channel),
            1,
            123,
            false,
        ),
        (
            "uppercase H does not block",
            vec![json!({"#h":[h]})],
            vec![vec!["H", "other"]],
            Some(channel),
            1,
            123,
            true,
        ),
        (
            "UUID selectors are strings",
            vec![json!({"#h":[h.to_uppercase()]})],
            vec![],
            Some(channel),
            1,
            123,
            false,
        ),
        (
            "empty h set",
            vec![json!({"#h":[]})],
            vec![],
            Some(channel),
            1,
            123,
            false,
        ),
        (
            "empty h selector and content",
            vec![json!({"#h":[""]})],
            vec![vec!["h", ""]],
            Some(channel),
            1,
            123,
            true,
        ),
        (
            "search and limit external",
            vec![json!({"search":"absent","limit":0})],
            vec![],
            None,
            1,
            123,
            true,
        ),
    ];
    for (name, raw_filters, tags, channel, kind, time, expected) in cases {
        let event: Event = serde_json::from_value(json!({
            "id":"01".repeat(32),"pubkey":AUTHOR,"created_at":time,"kind":kind,
            "tags":tags,"content":"fixture","sig":"00".repeat(64)
        }))
        .unwrap();
        let stored = StoredEvent::with_received_at(
            event,
            Utc.timestamp_opt(123, 0).single().unwrap(),
            channel,
            false,
        );
        let filters: Vec<Filter> = raw_filters
            .into_iter()
            .map(|f| serde_json::from_value(f).unwrap())
            .collect();
        assert_eq!(
            filters_match(&filters, &stored),
            expected,
            "historical: {name}"
        );
        let selectors = crate::prepared_filter::prepare_filters(&filters);
        let projected = crate::prepared_filter::prepare_event(&stored);
        assert_eq!(
            crate::prepared_filter::filters_match(&selectors, &projected),
            expected,
            "generated: {name}"
        );
    }
}

#[test]
fn generated_matcher_agrees_with_typed_reference_on_composed_inputs() {
    fn next(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }
    let mut seed = 0x987654321abcdefu64;
    let channel = Uuid::from_u128(0xa0112233445546778899aabbccdd0042);
    let h = channel.to_string();
    let keys = ["h", "H", "e", "t", "p", "", "é"];
    let values = ["", "a", "b", h.as_str(), "é", "e\u{301}"];
    for case in 0..10_000 {
        let mut tags = Vec::new();
        for _ in 0..next(&mut seed) % 12 {
            let mut tag = vec![keys[next(&mut seed) as usize % keys.len()]];
            if !next(&mut seed).is_multiple_of(3) {
                tag.push(values[next(&mut seed) as usize % values.len()]);
            }
            if next(&mut seed).is_multiple_of(3) {
                tag.push("later field");
            }
            tags.push(tag);
        }
        let kind = (next(&mut seed) % 3) as u16;
        let time = if next(&mut seed).is_multiple_of(5) {
            u64::MAX
        } else {
            next(&mut seed) % 3
        };
        let event: Event = serde_json::from_value(
            json!({"id":"01".repeat(32),"pubkey":AUTHOR,"created_at":time,
            "kind":kind,"tags":tags,"content":"fixture","sig":"00".repeat(64)}),
        )
        .unwrap();
        let stored = StoredEvent::with_received_at(
            event,
            Utc.timestamp_opt(123, 0).single().unwrap(),
            if next(&mut seed).is_multiple_of(2) {
                Some(channel)
            } else {
                None
            },
            false,
        );
        let mut filters = Vec::new();
        for _ in 0..next(&mut seed) % 4 {
            let mut f = json!({});
            for key in ["#h", "#H", "#e", "#t", "#p"] {
                if next(&mut seed).is_multiple_of(3) {
                    let selected: Vec<_> = (0..next(&mut seed) % 4)
                        .map(|_| values[next(&mut seed) as usize % values.len()])
                        .collect();
                    f[key] = json!(selected);
                }
            }
            if next(&mut seed).is_multiple_of(3) {
                f["kinds"] = json!(if next(&mut seed).is_multiple_of(4) {
                    vec![]
                } else {
                    vec![(next(&mut seed) % 3) as u16]
                });
            }
            for key in ["ids", "authors"] {
                if next(&mut seed).is_multiple_of(3) {
                    let value = if key == "ids" {
                        "01".repeat(32)
                    } else {
                        AUTHOR.to_string()
                    };
                    f[key] = json!(if next(&mut seed).is_multiple_of(3) {
                        vec![]
                    } else {
                        vec![value]
                    });
                }
            }
            if next(&mut seed).is_multiple_of(3) {
                f["since"] = json!(next(&mut seed) % 3);
            }
            if next(&mut seed).is_multiple_of(3) {
                f["until"] = json!(next(&mut seed) % 3);
            }
            filters.push(serde_json::from_value::<Filter>(f).unwrap());
        }
        let projected = crate::prepared_filter::prepare_event(&stored);
        let selectors = crate::prepared_filter::prepare_filters(&filters);
        assert_eq!(
            crate::prepared_filter::filters_match(&selectors, &projected),
            filters_match(&filters, &stored),
            "case {case}"
        );
    }
}
