use nostr::{EventBuilder, Kind};

/// Kind 0 — NIP-01 profile metadata (full snapshot).
pub fn build_profile(
    display_name: Option<&str>,
    name: Option<&str>,
    picture: Option<&str>,
    about: Option<&str>,
    nip05: Option<&str>,
) -> Result<EventBuilder, String> {
    let mut map = serde_json::Map::new();
    if let Some(v) = display_name {
        map.insert("display_name".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = name {
        map.insert("name".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = picture {
        map.insert("picture".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = about {
        map.insert("about".into(), serde_json::Value::String(v.into()));
    }
    if let Some(v) = nip05 {
        map.insert("nip05".into(), serde_json::Value::String(v.into()));
    }
    let content = serde_json::Value::Object(map).to_string();
    Ok(EventBuilder::new(Kind::Custom(0), content))
}

/// Partial update for an existing kind:0 metadata object.
///
/// Kind:0 events are replaceable full snapshots, so callers must start from
/// the complete prior object and patch only fields explicitly supplied by the
/// user. `None` preserves a field, a non-empty string sets its trimmed value,
/// and an empty/whitespace-only string removes it.
#[derive(Default)]
pub struct ProfileMetadataPatch<'a> {
    pub display_name: Option<&'a str>,
    pub name: Option<&'a str>,
    pub picture: Option<&'a str>,
    pub about: Option<&'a str>,
    pub nip05: Option<&'a str>,
}

pub fn build_patched_profile(
    mut metadata: serde_json::Map<String, serde_json::Value>,
    patch: ProfileMetadataPatch<'_>,
) -> Result<EventBuilder, String> {
    patch_profile_field(&mut metadata, "display_name", patch.display_name);
    patch_profile_field(&mut metadata, "name", patch.name);
    patch_profile_field(&mut metadata, "picture", patch.picture);
    patch_profile_field(&mut metadata, "about", patch.about);
    patch_profile_field(&mut metadata, "nip05", patch.nip05);

    Ok(EventBuilder::new(
        Kind::Custom(0),
        serde_json::Value::Object(metadata).to_string(),
    ))
}

fn patch_profile_field(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    value: Option<&str>,
) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if value.is_empty() {
        metadata.remove(field);
    } else {
        metadata.insert(field.to_string(), serde_json::Value::String(value.into()));
    }
}
