//! The synced channel-sections document (kind:30078, d=`channel-sections`).
//!
//! Wire-compatible with desktop `channelSectionsSync.ts`, `sidebarLaneStore.ts`
//! and `sidebarLwwMap.ts`: a tree of per-field last-writer-wins registers
//! `[v, dev, value]` under `meta`, plus the projected `sections`/`assignments`
//! fields that meta-less clients (mobile) read and write.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

/// Register tree: `{"s": {section_id: {name, icon, order, live}}, "a": {channel_id: section_id|null}}`.
pub(crate) type Tree = Map<String, Value>;

/// Live-section cap shared with desktop; larger documents are never published.
pub(crate) const MAX_SECTIONS: usize = 100;
/// Live-assignment cap shared with desktop.
pub(crate) const MAX_ASSIGNMENTS: usize = 1_000;

/// Device id desktop stamps on registers imported from meta-less payloads.
const LEGACY_DEV: &str = "0000000000000000";
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// One live section, densely renumbered by `(order, id)`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Section {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub order: usize,
}

/// The view every client renders: live sections and assignments to them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct Projection {
    pub sections: Vec<Section>,
    pub assignments: BTreeMap<String, String>,
}

impl Projection {
    pub(crate) fn within_limits(&self) -> bool {
        self.sections.len() <= MAX_SECTIONS && self.assignments.len() <= MAX_ASSIGNMENTS
    }
}

/// `Number.isSafeInteger`, accepting integral floats such as `1.0`.
fn safe_int(value: &Value) -> Option<i64> {
    let n = match value.as_i64() {
        Some(n) => n,
        None => {
            let f = value.as_f64()?;
            if f.fract() != 0.0 || f.abs() > MAX_SAFE_INTEGER as f64 {
                return None;
            }
            f as i64
        }
    };
    (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER)
        .contains(&n)
        .then_some(n)
}

fn is_dev(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| s.len() == 16 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

fn valid_reg(node: &Value, leaf: Leaf) -> Option<Value> {
    let reg = node.as_array().filter(|reg| reg.len() == 3)?;
    let v = safe_int(&reg[0]).filter(|v| *v >= 0)?;
    (is_dev(&reg[1]) && leaf(&reg[2])).then(|| json!([v, reg[1], reg[2]]))
}

fn is_string(value: &Value) -> bool {
    value.is_string()
}
fn is_string_or_null(value: &Value) -> bool {
    value.is_string() || value.is_null()
}
fn is_order(value: &Value) -> bool {
    safe_int(value).is_some()
}
fn is_bool(value: &Value) -> bool {
    value.is_boolean()
}

type Leaf = fn(&Value) -> bool;

const SECTION_FIELDS: [(&str, Leaf); 4] = [
    ("name", is_string),
    ("icon", is_string_or_null),
    ("order", is_order),
    ("live", is_bool),
];

/// Own entries of a JSON object, skipping the `__proto__` key desktop drops.
fn entries(node: &Value) -> impl Iterator<Item = (&String, &Value)> {
    node.as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| key.as_str() != "__proto__")
}

/// Keeps only well-formed registers; an invalid one is dropped while its
/// valid siblings survive (desktop `validTree` with the sections shape).
fn valid_tree(fields: &Value) -> Option<Tree> {
    fields.as_object()?;
    let mut tree = Tree::new();
    if let Some(s) = fields.get("s").filter(|s| s.is_object()) {
        let mut sections = Map::new();
        for (id, node) in entries(s) {
            if !node.is_object() {
                continue;
            }
            let mut out = Map::new();
            for (field, leaf) in SECTION_FIELDS {
                if let Some(reg) = node.get(field).and_then(|n| valid_reg(n, leaf)) {
                    out.insert(field.to_string(), reg);
                }
            }
            sections.insert(id.clone(), Value::Object(out));
        }
        tree.insert("s".into(), Value::Object(sections));
    }
    if let Some(a) = fields.get("a").filter(|a| a.is_object()) {
        let assignments = entries(a)
            .filter_map(|(id, node)| Some((id.clone(), valid_reg(node, is_string_or_null)?)))
            .collect();
        tree.insert("a".into(), Value::Object(assignments));
    }
    Some(tree)
}

/// JS `Math.round`, as an integer when it is a safe one.
fn js_round(f: f64) -> Value {
    let rounded = (f + 0.5).floor();
    if rounded.abs() <= MAX_SAFE_INTEGER as f64 {
        json!(rounded as i64)
    } else {
        json!(rounded)
    }
}

/// Registers for a meta-less (legacy/mobile) payload, every value stamped at `legacy_v`.
fn from_legacy(json: &Value, legacy_v: i64) -> Value {
    let stamp = |value: Value| json!([legacy_v, LEGACY_DEV, value]);
    let mut s = Map::new();
    for entry in json
        .get("sections")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(name), Some(order)) = (
            entry.get("id").and_then(Value::as_str),
            entry.get("name").and_then(Value::as_str),
            entry.get("order").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let icon = entry
            .get("icon")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|icon| !icon.is_empty());
        s.insert(
            id.to_string(),
            json!({
                "name": stamp(json!(name)),
                "icon": stamp(json!(icon)),
                "order": stamp(js_round(order)),
                "live": stamp(json!(true)),
            }),
        );
    }
    let a: Map<String, Value> = entries(json.get("assignments").unwrap_or(&Value::Null))
        .filter(|(_, target)| target.as_str().is_some_and(|t| s.contains_key(t)))
        .map(|(id, target)| (id.clone(), stamp(target.clone())))
        .collect();
    json!({ "s": s, "a": a })
}

/// Decodes a `version: 1` payload, importing a meta-less one at `legacy_v`.
/// Returns `None` when unreadable, including an unknown `meta.v`.
pub(crate) fn decode(json: &Value, legacy_v: i64) -> Option<Tree> {
    let is_one = |value: Option<&Value>| value.and_then(safe_int) == Some(1);
    if !is_one(json.get("version")) {
        return None;
    }
    match json.get("meta") {
        None => valid_tree(&from_legacy(json, legacy_v)),
        Some(meta) if is_one(meta.get("v")) => {
            let mut fields = meta.as_object()?.clone();
            fields.remove("v");
            valid_tree(&Value::Object(fields))
        }
        Some(_) => None,
    }
}

fn val<'a>(node: &'a Value, field: &str) -> Option<&'a Value> {
    node.get(field)?.as_array()?.get(2)
}

/// Live sections by `(order, id)` with their stored `order`.
fn ranked(tree: &Tree) -> Vec<(String, String, Option<String>, i64)> {
    let mut out: Vec<_> = tree
        .get("s")
        .map(entries)
        .into_iter()
        .flatten()
        .filter(|(_, node)| val(node, "live") == Some(&json!(true)))
        .filter_map(|(id, node)| {
            let name = val(node, "name")?.as_str()?.to_string();
            let icon = val(node, "icon")
                .and_then(Value::as_str)
                .filter(|icon| !icon.is_empty())
                .map(str::to_string);
            let order = val(node, "order").and_then(safe_int).unwrap_or(0);
            Some((id.clone(), name, icon, order))
        })
        .collect();
    // JS compares strings by UTF-16 code unit.
    out.sort_by(|a, b| {
        a.3.cmp(&b.3)
            .then_with(|| a.0.encode_utf16().cmp(b.0.encode_utf16()))
    });
    out
}

/// Highest stored `order` among live sections, or -1 when there are none.
fn max_live_order(tree: &Tree) -> i64 {
    ranked(tree).iter().map(|s| s.3).max().unwrap_or(-1)
}

pub(crate) fn project(tree: &Tree) -> Projection {
    let sections: Vec<Section> = ranked(tree)
        .into_iter()
        .enumerate()
        .map(|(order, (id, name, icon, _))| Section {
            id,
            name,
            icon,
            order,
        })
        .collect();
    let assignments = tree
        .get("a")
        .map(entries)
        .into_iter()
        .flatten()
        .filter_map(|(channel, reg)| {
            let target = reg.as_array()?.get(2)?.as_str()?;
            sections
                .iter()
                .any(|s| s.id == target)
                .then(|| (channel.clone(), target.to_string()))
        })
        .collect();
    Projection {
        sections,
        assignments,
    }
}

/// The plaintext document: projected legacy fields plus the register tree.
pub(crate) fn encode(tree: &Tree) -> Value {
    let Projection {
        sections,
        assignments,
    } = project(tree);
    let mut meta = tree.clone();
    meta.insert("v".into(), json!(1));
    json!({
        "version": 1,
        "sections": sections,
        "assignments": assignments,
        "meta": meta,
    })
}

fn max_version(node: &Value) -> i64 {
    match node {
        Value::Array(reg) => reg.first().and_then(safe_int).unwrap_or(0),
        Value::Object(map) => map.values().map(max_version).max().unwrap_or(0),
        _ => 0,
    }
}

/// The version and device every register written by one edit shares.
#[derive(Debug, Clone)]
pub(crate) struct Stamp {
    pub now_ms: i64,
    pub dev: String,
}

/// One edit against a tree. Each call to [`Edit::set`] that changes a leaf
/// writes a register stamped `max(now, newest + 1)`, like desktop `setRegs`.
pub(crate) struct Edit<'a> {
    tree: &'a mut Tree,
    reg: (i64, String),
}

impl<'a> Edit<'a> {
    pub(crate) fn new(tree: &'a mut Tree, stamp: &Stamp) -> Self {
        let newest = tree.values().map(max_version).max().unwrap_or(0);
        let v = stamp.now_ms.max(newest + 1);
        Self {
            tree,
            reg: (v, stamp.dev.clone()),
        }
    }

    fn current(&self, path: &[&str]) -> Option<&Value> {
        let (first, rest) = path.split_first()?;
        let mut node = self.tree.get(*first)?;
        for key in rest {
            node = node.get(key)?;
        }
        node.as_array()?.get(2)
    }

    fn write(&mut self, parents: &[&str], leaf: &str, value: Value) {
        let reg = json!([self.reg.0, self.reg.1, value]);
        let mut node = &mut *self.tree;
        for key in parents {
            let child = node.entry(key.to_string()).or_insert_with(|| json!({}));
            if !child.is_object() {
                *child = json!({});
            }
            let Value::Object(map) = child else {
                return;
            };
            node = map;
        }
        node.insert(leaf.to_string(), reg);
    }

    /// Writes `value` at `path` unless it already holds that value.
    pub(crate) fn set(&mut self, path: &[&str], value: Value) {
        let Some((leaf, parents)) = path.split_last() else {
            return;
        };
        if self.current(path) != Some(&value) {
            self.write(parents, leaf, value);
        }
    }

    pub(crate) fn create_section(&mut self, id: &str, name: &str, icon: Option<&str>) {
        let order = max_live_order(self.tree) + 1;
        self.set(&["s", id, "name"], json!(name));
        self.set(&["s", id, "icon"], json!(icon));
        self.set(&["s", id, "order"], json!(order));
        self.set(&["s", id, "live"], json!(true));
    }

    /// Orders live sections `requested` first, then the rest in their current
    /// relative order. Any change rewrites every live `order` register.
    pub(crate) fn reorder(&mut self, requested: &[String]) {
        let live: Vec<String> = project(self.tree)
            .sections
            .into_iter()
            .map(|s| s.id)
            .collect();
        let mut ids: Vec<String> = Vec::with_capacity(live.len());
        for id in requested.iter().chain(&live) {
            if live.contains(id) && !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        let unchanged = ids
            .iter()
            .enumerate()
            .all(|(index, id)| self.current(&["s", id, "order"]) == Some(&json!(index)));
        if unchanged {
            return;
        }
        for (index, id) in ids.iter().enumerate() {
            self.write(&["s", id], "order", json!(index));
        }
    }

    /// Clears an assignment that exists (as a value or tombstone).
    pub(crate) fn unassign(&mut self, channel_id: &str) {
        let exists = self.tree.get("a").and_then(|a| a.get(channel_id)).is_some();
        if exists {
            self.set(&["a", channel_id], Value::Null);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV: &str = "0123456789abcdef";

    fn stamp(now_ms: i64) -> Stamp {
        Stamp {
            now_ms,
            dev: DEV.into(),
        }
    }

    fn reg(v: i64, value: Value) -> Value {
        json!([v, DEV, value])
    }

    /// A document in the shape desktop `encodeDoc` publishes.
    fn desktop_doc() -> Value {
        json!({
            "version": 1,
            "sections": [],
            "assignments": {},
            "meta": {
                "v": 1,
                "s": {
                    "b": {"name": reg(5, json!("Beta")), "icon": reg(5, Value::Null),
                          "order": reg(5, json!(1)), "live": reg(5, json!(true))},
                    "a": {"name": reg(4, json!("Alpha")), "icon": reg(4, json!("🚀")),
                          "order": reg(4, json!(0)), "live": reg(4, json!(true))},
                    "dead": {"name": reg(3, json!("Gone")), "icon": reg(3, Value::Null),
                             "order": reg(3, json!(2)), "live": reg(6, json!(false))},
                    "bad": {"name": [7, "not-a-dev", "Bad"], "order": reg(7, json!(3)),
                            "live": reg(7, json!(true))},
                },
                "a": {
                    "c1": reg(8, json!("a")),
                    "c2": reg(8, json!("dead")),
                    "c3": reg(9, Value::Null),
                },
                "future": reg(1, json!("ignored")),
            },
        })
    }

    #[test]
    fn decodes_desktop_document_and_projects_live_state() {
        let tree = decode(&desktop_doc(), 0).expect("readable");
        let view = project(&tree);
        let names: Vec<_> = view
            .sections
            .iter()
            .map(|s| (s.name.as_str(), s.order))
            .collect();
        // `bad` keeps its valid registers but has no name, so it is not live.
        assert_eq!(names, [("Alpha", 0), ("Beta", 1)]);
        assert_eq!(view.sections[0].icon.as_deref(), Some("🚀"));
        assert_eq!(
            view.assignments,
            BTreeMap::from([("c1".to_string(), "a".to_string())])
        );
        assert!(!tree.contains_key("future"));
    }

    #[test]
    fn unreadable_payloads_are_rejected() {
        for payload in [
            json!({"version": 2, "meta": {"v": 1}}),
            json!({"version": 1, "meta": {"v": 2}}),
            json!({"meta": {"v": 1}}),
            json!("text"),
        ] {
            assert!(decode(&payload, 0).is_none(), "{payload}");
        }
        let bad_version = json!({"version": 1.0, "meta": {"v": 1, "s": {"x": {
            "order": [i64::MIN, DEV, 0], "live": [-1, DEV, true], "name": [1, DEV, "X"]}}}});
        let tree = decode(&bad_version, 0).expect("readable");
        assert_eq!(tree["s"]["x"], json!({"name": [1, DEV, "X"]}));
    }

    #[test]
    fn imports_mobile_flat_payload_at_the_event_version() {
        let mobile = json!({
            "version": 1,
            "sections": [
                {"id": "x", "name": "Team", "icon": "  ", "order": 1.6},
                {"id": "y", "name": "Ops", "order": 0},
                {"name": "no id", "order": 0},
            ],
            "assignments": {"c1": "x", "c2": "missing"},
        });
        let tree = decode(&mobile, 42_000).expect("readable");
        assert_eq!(tree["s"]["x"]["order"], json!([42_000, LEGACY_DEV, 2]));
        assert_eq!(tree["s"]["x"]["icon"], json!([42_000, LEGACY_DEV, null]));
        let view = project(&tree);
        let names: Vec<_> = view.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Ops", "Team"]);
        assert_eq!(view.assignments.len(), 1);
    }

    #[test]
    fn encode_round_trips_and_carries_the_projection() {
        let tree = decode(&desktop_doc(), 0).expect("readable");
        let doc = encode(&tree);
        assert_eq!(doc["meta"]["v"], json!(1));
        assert_eq!(doc["sections"][0]["name"], json!("Alpha"));
        assert!(doc["sections"][1].get("icon").is_none());
        assert_eq!(doc["assignments"], json!({"c1": "a"}));
        assert_eq!(decode(&doc, 0).expect("readable"), tree);
    }

    #[test]
    fn edits_share_one_stamp_newer_than_every_register() {
        let mut tree = decode(&desktop_doc(), 0).expect("readable");
        let mut edit = Edit::new(&mut tree, &stamp(1));
        edit.create_section("n", "New", None);
        edit.set(&["a", "c9"], json!("n"));
        assert_eq!(tree["s"]["n"]["order"], reg(10, json!(2)));
        assert_eq!(tree["a"]["c9"], reg(10, json!("n")));

        let mut edit = Edit::new(&mut tree, &stamp(500));
        edit.set(&["s", "n", "name"], json!("New"));
        assert_eq!(
            tree["s"]["n"]["name"],
            reg(10, json!("New")),
            "unchanged leaf"
        );
    }

    #[test]
    fn reorder_puts_requested_first_and_keeps_the_rest() {
        let mut tree = Tree::new();
        let mut edit = Edit::new(&mut tree, &stamp(1));
        for id in ["a", "b", "c"] {
            edit.create_section(id, id, None);
        }
        let mut edit = Edit::new(&mut tree, &stamp(2));
        edit.reorder(&["c".into(), "dead".into(), "c".into()]);
        let ids: Vec<_> = project(&tree).sections.into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["c", "a", "b"]);
        assert_eq!(tree["s"]["a"]["order"], json!([2, DEV, 1]));

        let before = tree.clone();
        Edit::new(&mut tree, &stamp(3)).reorder(&["c".into(), "a".into()]);
        assert_eq!(
            tree, before,
            "a reorder to the current order writes nothing"
        );
    }

    #[test]
    fn unassign_tombstones_only_existing_assignments() {
        let mut tree = Tree::new();
        let mut edit = Edit::new(&mut tree, &stamp(1));
        edit.create_section("s1", "S", None);
        edit.set(&["a", "c1"], json!("s1"));
        let mut edit = Edit::new(&mut tree, &stamp(2));
        edit.unassign("c1");
        edit.unassign("never");
        assert_eq!(tree["a"]["c1"], json!([2, DEV, null]));
        assert!(tree["a"].get("never").is_none());
    }
}
