//! Synthetic, protocol-shaped inputs. This matrix is not a measured traffic mix.
use buzz_core::StoredEvent;
use chrono::{TimeZone, Utc};
use nostr::Filter;
use serde_json::{json, Value};
use std::sync::OnceLock;
use uuid::Uuid;

pub const AUTHOR: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
pub const OTHER_AUTHOR: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";
pub struct Case {
    pub name: String,
    pub filters: Vec<Filter>,
    pub event: StoredEvent,
    pub expected: bool,
}

/// Deterministic canonical UUIDv4, not a zero/versionless synthetic UUID.
pub fn channel(i: usize) -> Uuid {
    Uuid::from_u128(0x00112233445546778899000000000000 + i as u128)
}
pub fn event_id(i: usize) -> String {
    format!("{i:064x}")
}
pub fn mixed_tags(count: usize) -> Vec<Vec<String>> {
    (0..count)
        .map(|i| match i % 4 {
            0 => vec!["e".into(), event_id(10000 + i)],
            1 => vec!["h".into(), channel(10000 + i).to_string()],
            2 => vec!["t".into(), format!("topic-{i}")],
            _ => vec!["p".into(), AUTHOR.into()],
        })
        .collect()
}
pub fn event(tags: Vec<Vec<String>>, channel: Option<Uuid>) -> StoredEvent {
    let event = serde_json::from_value(json!({
        "id": "01".repeat(32), "pubkey": AUTHOR, "created_at": 123,
        "kind": 1, "tags": tags, "content": "fixture", "sig": "00".repeat(64)
    }))
    .unwrap();
    StoredEvent::with_received_at(
        event,
        Utc.timestamp_opt(123, 0).single().unwrap(),
        channel,
        false,
    )
}
pub fn filter(value: Value) -> Filter {
    serde_json::from_value(value).unwrap()
}

pub fn cases() -> &'static [Case] {
    static CASES: OnceLock<Vec<Case>> = OnceLock::new();
    CASES.get_or_init(|| {
        let mut cases = Vec::new();
        for (key,tag_count,value_count) in [("e",4,1),("e",4,20),("e",16,1),("e",16,20),
            ("e",32,50),("h",4,1),("h",16,50),("h",32,50),("h",32,128)] {
            for position in ["early", "late", "miss"] {
                let values: Vec<String> = (0..value_count).map(|i| if key=="h" {channel(i).to_string()} else {event_id(i)}).collect();
                let mut tags = mixed_tags(tag_count);
                if position == "early" { tags[0] = vec![key.into(), values[0].clone()]; }
                if position == "late" { tags[tag_count-1] = vec![key.into(),values[value_count-1].clone()]; }
                cases.push(Case { name: format!("{key}_tags{tag_count}_values{value_count}_{position}"),
                    filters: vec![filter(json!({"kinds":[1],format!("#{key}"):values}))], event: event(tags,None), expected: position != "miss" });
            }
        }
        let tags = mixed_tags(16);
        for (name, constraint) in [
            ("kind_miss",json!({"kinds":[9]})),
            ("id_miss",json!({"ids":["02".repeat(32)]})),
            ("author_miss",json!({"authors":[OTHER_AUTHOR]})),
            ("since_miss",json!({"since":124})),
            ("until_miss",json!({"until":122})),
            ("all_scalar_hit",json!({"ids":["01".repeat(32)],"authors":[AUTHOR],"kinds":[1],"since":123,"until":123}))
        ] { cases.push(Case { name:name.into(),filters:vec![filter(constraint)],event:event(tags.clone(),None),expected:name=="all_scalar_hit" }); }
        for position in ["early","late","miss"] {
            let mut filters: Vec<Filter> = (0..10).map(|i| filter(json!({"kinds":[1],"#e":[event_id(i)]}))).collect();
            if position == "early" { filters[0]=filter(json!({"kinds":[1],"#e":[event_id(10000)]})); }
            if position == "late" { filters[9]=filter(json!({"kinds":[1],"#e":[event_id(10012)]})); }
            cases.push(Case { name:format!("or10_{position}"),filters,event:event(tags.clone(),None),expected:position!="miss" });
        }
        let ch=channel(42);
        let without_h=vec![vec!["t".into(),"topic".into()]];
        cases.push(Case { name:"channel_fallback".into(),filters:vec![filter(json!({"#h":[ch.to_string()]}))],event:event(without_h,Some(ch)),expected:true });
        cases.push(Case { name:"channel_explicit_miss".into(),filters:vec![filter(json!({"#h":[ch.to_string()]}))],event:event(vec![vec!["h".into(),channel(43).to_string()]],Some(ch)),expected:false });
        cases.push(Case { name:"empty_filters".into(),filters:vec![],event:event(vec![],None),expected:false });
        cases.push(Case { name:"empty_object".into(),filters:vec![Filter::new()],event:event(vec![],None),expected:true });
        cases
    })
}
