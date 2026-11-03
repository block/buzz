//! Guarded task archive and restore: the `nexus.task` hierarchy profile, the
//! kind:9002 lifecycle guard tags and the lifecycle digest.
//!
//! A guarded state request carries `["archived", "true"|"false"]`,
//! `["scope", "target"|"subtree"]` and `["expect", hex(D)]`. A fence carries
//! `["lifecycle", "fence"]` and `["expect-target", <token>|"-"]`. The relay
//! enforces either the whole guard or refuses the request; it never applies a
//! guarded request as an unguarded one.
use crate::artifact::{canonical_uuid, ArtifactEnvelope, ArtifactOp};
use nostr::Event;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Artifact type of the records whose `parent` tags the relay validates.
pub const TASK_ARTIFACT_TYPE: &str = "nexus.task";

/// Domain separator, the first line of every lifecycle digest.
pub const DIGEST_DOMAIN: &str = "buzz.lifecycle.v1";

/// Digest text for a token that no lifecycle writer has set since the column
/// was added.
pub const NULL_TOKEN: &str = "-";

/// kind:9002 tags that change channel settings. A guarded request carries
/// none of them, so its event changes lifecycle state and nothing else.
const SETTINGS_TAGS: &[&str] = &[
    "name",
    "about",
    "topic",
    "purpose",
    "visibility",
    "ttl",
    "t",
    "join_role",
    "unlisted",
];

/// Tags whose presence makes a kind:9002 a guarded lifecycle request.
const GUARD_TAGS: &[&str] = &["lifecycle", "expect", "expect-target", "scope"];

/// The `d` of a channel's task record:
/// UUIDv5(RFC 4122 URL namespace, `"nexus.task:" + channel`).
pub fn task_d(channel: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("{TASK_ARTIFACT_TYPE}:{channel}").as_bytes(),
    )
}

/// The hierarchy fields of a validated `nexus.task` revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskEdge {
    /// The task's channel (`h`).
    pub channel: Uuid,
    /// The parent task's channel, or `None` for a root. Always `None` on a
    /// delete, which removes the edge.
    pub parent: Option<Uuid>,
}

/// The hierarchy fields of `nexus.task` content. A derived struct refuses a
/// repeated hierarchy key instead of keeping one of its values.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskContent {
    v: serde_json::Value,
    channel_id: String,
    /// `None` only when the key is absent: an explicit `null` is kept, and
    /// refused, rather than read as a root.
    #[serde(default, deserialize_with = "present")]
    parent_channel_id: Option<serde_json::Value>,
}

fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde::Deserialize::deserialize(deserializer).map(Some)
}

/// Validate the `nexus.task` profile of an artifact revision. Returns
/// `Ok(None)` for any other artifact type.
///
/// The relay parses only the hierarchy fields; the rest of the content stays
/// opaque. A revision must name its channel's derived `d`, carry content with
/// `v = 0` and `channelId = h`, and carry at most one `parent` tag that equals
/// the content's `parentChannelId`.
pub fn task_edge(event: &Event, env: &ArtifactEnvelope) -> Result<Option<TaskEdge>, &'static str> {
    if env.artifact_type != TASK_ARTIFACT_TYPE {
        return Ok(None);
    }
    if env.id != task_d(env.home) {
        return Err("nexus.task d must be derived from its channel");
    }
    if env.op == ArtifactOp::Delete {
        // The envelope already refuses content and extra tags on a delete.
        return Ok(Some(TaskEdge {
            channel: env.home,
            parent: None,
        }));
    }
    let mut parent_tag = None;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if parts.first().map(String::as_str) != Some("parent") {
            continue;
        }
        if parts.len() != 2 || parent_tag.is_some() {
            return Err("nexus.task allows one parent tag with exactly two elements");
        }
        parent_tag = Some(canonical_uuid(&parts[1])?);
    }
    let content: TaskContent = serde_json::from_str(&event.content)
        .map_err(|_| "nexus.task content is not a v0 record")?;
    if content.v.as_u64() != Some(0) || content.v.is_f64() {
        return Err("nexus.task content is not a v0 record");
    }
    if content.channel_id != env.home.to_string() {
        return Err("nexus.task content channelId must equal h");
    }
    let content_parent = match content.parent_channel_id {
        None => None,
        Some(serde_json::Value::String(value)) => Some(canonical_uuid(&value)?),
        Some(_) => return Err("nexus.task parentChannelId must be a UUID"),
    };
    if content_parent != parent_tag {
        return Err("nexus.task parent tag must equal content parentChannelId");
    }
    if parent_tag == Some(env.home) {
        return Err("a task cannot be its own parent");
    }
    Ok(Some(TaskEdge {
        channel: env.home,
        parent: parent_tag,
    }))
}

/// Which channels a guarded restore writes. Archive always uses `Subtree`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleScope {
    /// The target and its minimum archived ancestor chain.
    Target,
    /// The target, its minimum archived ancestor chain and its descendants.
    Subtree,
}

impl LifecycleScope {
    /// Wire and digest text.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Target => "target",
            Self::Subtree => "subtree",
        }
    }
}

/// A parsed kind:9002 lifecycle guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleGuard {
    /// Archive (`archive = true`) or restore, applied only if the digest of
    /// the current lifecycle state equals `expect`.
    State {
        /// `true` archives, `false` restores.
        archive: bool,
        /// Which channels the request writes.
        scope: LifecycleScope,
        /// The digest the client previewed.
        expect: [u8; 32],
    },
    /// Replace the target's token if it still equals `expect_target`
    /// (`None` for a token that was never set). Changes no other state.
    Fence {
        /// The token the client last read.
        expect_target: Option<Uuid>,
    },
}

/// Parse the lifecycle guard of a kind:9002. `Ok(None)` means the event
/// carries no guard tag. Any guard tag makes the whole guard mandatory: a
/// malformed or incomplete guard is an error, never an unguarded request.
pub fn parse_lifecycle_guard(event: &Event) -> Result<Option<LifecycleGuard>, &'static str> {
    let mut lifecycle = None;
    let mut expect = None;
    let mut expect_target = None;
    let mut scope = None;
    let mut archived = Vec::new();
    let mut targets = Vec::new();
    let mut guarded = false;
    let mut settings = false;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(name) = parts.first().map(String::as_str) else {
            continue;
        };
        if SETTINGS_TAGS.contains(&name) {
            settings = true;
        }
        // Kept whole: a guarded request refuses any other arity.
        if name == "archived" {
            archived.push(parts);
        }
        if name == "h" {
            targets.push(parts);
        }
        if !GUARD_TAGS.contains(&name) {
            continue;
        }
        guarded = true;
        if parts.len() != 2 {
            return Err("lifecycle tags must have exactly two elements");
        }
        let slot = match name {
            "lifecycle" => &mut lifecycle,
            "expect" => &mut expect,
            "expect-target" => &mut expect_target,
            _ => &mut scope,
        };
        if slot.replace(parts[1].as_str()).is_some() {
            return Err("lifecycle tags must occur once");
        }
    }
    if !guarded {
        return Ok(None);
    }
    // One unambiguous target; the generic `h` lookup takes the first match.
    match targets.as_slice() {
        [[_, target]] => {
            canonical_uuid(target)?;
        }
        _ => return Err("a lifecycle request requires one h tag with exactly two elements"),
    }
    if settings {
        return Err("a lifecycle request cannot change channel settings");
    }
    if let Some(kind) = lifecycle {
        if kind != "fence" {
            return Err("unknown lifecycle request");
        }
        if !archived.is_empty() || expect.is_some() || scope.is_some() {
            return Err("a lifecycle fence carries only expect-target");
        }
        let expect_target = match expect_target.ok_or("a lifecycle fence requires expect-target")? {
            NULL_TOKEN => None,
            token => Some(canonical_uuid(token)?),
        };
        return Ok(Some(LifecycleGuard::Fence { expect_target }));
    }
    if expect_target.is_some() {
        return Err("expect-target belongs to a lifecycle fence");
    }
    let archive = match archived.as_slice() {
        [[_, value]] if value == "true" => true,
        [[_, value]] if value == "false" => false,
        _ => return Err("a guarded lifecycle request requires one archived tag"),
    };
    let scope = match scope {
        Some("target") => LifecycleScope::Target,
        Some("subtree") => LifecycleScope::Subtree,
        _ => return Err("a guarded lifecycle request requires scope target or subtree"),
    };
    if archive && scope == LifecycleScope::Target {
        return Err("archive scope must be subtree");
    }
    let expect = expect.ok_or("a guarded lifecycle request requires expect")?;
    let mut digest = [0u8; 32];
    if expect.len() != 64
        || !expect
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || hex::decode_to_slice(expect, &mut digest).is_err()
    {
        return Err("expect must be 64 lowercase hex characters");
    }
    Ok(Some(LifecycleGuard::State {
        archive,
        scope,
        expect: digest,
    }))
}

/// A channel and its lifecycle token, as bound by a digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TokenEntry {
    /// The channel.
    pub channel: Uuid,
    /// Its token, or `None` if never set.
    pub token: Option<Uuid>,
}

impl TokenEntry {
    fn line(&self) -> String {
        match self.token {
            Some(token) => format!("{} {token}", self.channel),
            None => format!("{} {NULL_TOKEN}", self.channel),
        }
    }
}

/// The lifecycle digest D of a guarded state request: SHA-256 over these
/// UTF-8 lines joined by `\n`, with no trailing newline:
///
/// 1. [`DIGEST_DOMAIN`]
/// 2. `archive` or `restore`
/// 3. the scope (`target` or `subtree`)
/// 4. the target channel
/// 5. `compared`: `<channel> <token>` of the first active ancestor above the
///    written chain, or `root` when there is none
/// 6. one `<channel> <token>` line per affected channel, sorted by channel
///
/// UUIDs are lowercase and hyphenated; a token that was never set is `-`.
pub fn lifecycle_digest(
    archive: bool,
    scope: LifecycleScope,
    target: Uuid,
    compared: Option<TokenEntry>,
    affected: &[TokenEntry],
) -> [u8; 32] {
    let mut affected = affected.to_vec();
    affected.sort();
    let mut lines = vec![
        DIGEST_DOMAIN.to_string(),
        if archive { "archive" } else { "restore" }.to_string(),
        scope.as_str().to_string(),
        target.to_string(),
        compared.map_or_else(|| "root".to_string(), |entry| entry.line()),
    ];
    lines.extend(affected.iter().map(TokenEntry::line));
    Sha256::digest(lines.join("\n").as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event(kind: u16, tags: &[&[&str]], content: &str) -> Event {
        let tags = tags
            .iter()
            .map(|t| Tag::parse(t.iter().copied()).unwrap())
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn task_d_matches_the_nexus_derivation() {
        // Vector from Nexus `taskD` (src/store/tasks.ts:363-372, 62ceb64):
        // UUIDv5 over the URL namespace and "nexus.task:<channel>".
        let channel = Uuid::parse_str("8f5b6a1e-2f0d-4c1a-9b3e-7d4c2a1f0e9b").unwrap();
        let d = task_d(channel);
        assert_eq!(d.get_version_num(), 5);
        assert_eq!(
            d,
            Uuid::new_v5(
                &Uuid::parse_str("6ba7b811-9dad-11d1-80b4-00c04fd430c8").unwrap(),
                b"nexus.task:8f5b6a1e-2f0d-4c1a-9b3e-7d4c2a1f0e9b"
            )
        );
        assert_eq!(d.to_string(), "c0b4b794-a2d3-5bc2-b435-74db64e475cb");
    }

    #[test]
    fn lifecycle_digest_pins_its_byte_encoding() {
        let id = |n: u8| Uuid::from_bytes([n; 16]);
        let affected = [
            TokenEntry {
                channel: id(3),
                token: Some(id(0xa3)),
            },
            TokenEntry {
                channel: id(2),
                token: None,
            },
        ];
        let compared = TokenEntry {
            channel: id(1),
            token: Some(id(0xa1)),
        };
        let text = "buzz.lifecycle.v1\nrestore\nsubtree\n02020202-0202-0202-0202-020202020202\n\
01010101-0101-0101-0101-010101010101 a1a1a1a1-a1a1-a1a1-a1a1-a1a1a1a1a1a1\n\
02020202-0202-0202-0202-020202020202 -\n\
03030303-0303-0303-0303-030303030303 a3a3a3a3-a3a3-a3a3-a3a3-a3a3a3a3a3a3";
        let expected: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        let digest = lifecycle_digest(
            false,
            LifecycleScope::Subtree,
            id(2),
            Some(compared),
            &affected,
        );
        assert_eq!(digest, expected);
        assert_eq!(
            hex::encode(digest),
            "3821bc3142cdbc0a678052580d5e103553669eb11ebd956d1678411d65938fd7"
        );
        // Every bound field changes the digest.
        let root = lifecycle_digest(false, LifecycleScope::Subtree, id(2), None, &affected);
        let target = lifecycle_digest(
            false,
            LifecycleScope::Target,
            id(2),
            Some(compared),
            &affected,
        );
        let archive = lifecycle_digest(
            true,
            LifecycleScope::Subtree,
            id(2),
            Some(compared),
            &affected,
        );
        let bumped = lifecycle_digest(
            false,
            LifecycleScope::Subtree,
            id(2),
            Some(compared),
            &[
                affected[0],
                TokenEntry {
                    channel: id(2),
                    token: Some(id(9)),
                },
            ],
        );
        for other in [root, target, archive, bumped] {
            assert_ne!(other, digest);
        }
    }

    /// Golden vectors for the client: archive of a subtree with an already
    /// archived child, restore of a target with its archived ancestor chain,
    /// and archive of a root.
    #[test]
    fn lifecycle_digest_golden_vectors() {
        let id = |n: u8| Uuid::from_bytes([n; 16]);
        let entry = |channel: u8, token: Option<u8>| TokenEntry {
            channel: id(channel),
            token: token.map(id),
        };
        // 1 -> 2 (target) -> {3 -> 5, 4 archived}: affected 2, 3, 5.
        let archive = lifecycle_digest(
            true,
            LifecycleScope::Subtree,
            id(2),
            Some(entry(1, Some(0xa1))),
            &[entry(5, None), entry(2, Some(0xa2)), entry(3, Some(0xa3))],
        );
        assert_eq!(
            hex::encode(archive),
            "e9fa8e584e36a45a6773703a48536e276fee69015545bbc3b40169a56cbaeef7"
        );
        // 1 active -> 2 archived -> 3 archived -> 4 archived (target):
        // affected 2, 3, 4; compared 1.
        let restore = lifecycle_digest(
            false,
            LifecycleScope::Target,
            id(4),
            Some(entry(1, Some(0xa1))),
            &[entry(4, Some(0xb4)), entry(2, Some(0xb2)), entry(3, None)],
        );
        assert_eq!(
            hex::encode(restore),
            "32a700a437403b0695e028542a50e74705b4416095dee22366a8584fd7ce4fdd"
        );
        let root = lifecycle_digest(
            true,
            LifecycleScope::Subtree,
            id(1),
            None,
            &[entry(1, None)],
        );
        assert_eq!(
            hex::encode(root),
            "948f7d0fd8015aa8354f4f6f193dfc60442ca6bcb485d53b7a36e9a4738eedc7"
        );
        // The Nexus client's independently computed vectors.
        let v4 = |text: &str| Uuid::parse_str(text).unwrap();
        let at = |channel: &str, token: Option<&str>| TokenEntry {
            channel: v4(channel),
            token: token.map(v4),
        };
        let (c3, c4, c5, c6) = (
            "33333333-3333-4333-8333-333333333333",
            "44444444-4444-4444-8444-444444444444",
            "55555555-5555-4555-8555-555555555555",
            "66666666-6666-4666-8666-666666666666",
        );
        let client = [
            lifecycle_digest(
                true,
                LifecycleScope::Subtree,
                v4(c3),
                None,
                &[
                    at(c5, None),
                    at(c3, Some("bbbbbbbb-bbbb-4bbb-abbb-bbbbbbbbbbbb")),
                    at(c4, Some("cccccccc-cccc-4ccc-accc-cccccccccccc")),
                ],
            ),
            lifecycle_digest(
                false,
                LifecycleScope::Target,
                v4(c5),
                Some(at(
                    "11111111-1111-4111-8111-111111111111",
                    Some("dddddddd-dddd-4ddd-addd-dddddddddddd"),
                )),
                &[
                    at(c5, Some("eeeeeeee-eeee-4eee-aeee-eeeeeeeeeeee")),
                    at(c3, None),
                ],
            ),
            lifecycle_digest(
                false,
                LifecycleScope::Subtree,
                v4(c3),
                None,
                &[at(c6, None), at(c3, None)],
            ),
        ];
        assert_eq!(
            client.map(hex::encode),
            [
                "862bfa3b97e3e10856a18a942e547b386e2582e407d2b2c1601bfff7c3cb6db5",
                "1dc896768fc8b72ccf195aa4eb9fd4689bbbb99e8031d24932932753d56a56bc",
                "6a93b6009926c8e7bd731f926e96a02020fb4cf33084ef76a5517ea1a27990f2",
            ]
        );
    }

    #[test]
    fn guard_parsing_never_degrades_to_unguarded() {
        let h = Uuid::new_v4().to_string();
        let hex = "ab".repeat(32);
        let token = Uuid::new_v4().to_string();
        let ok = |tags: &[&[&str]]| parse_lifecycle_guard(&event(9002, tags, ""));
        assert_eq!(ok(&[&["h", &h], &["archived", "true"]]), Ok(None));
        assert!(matches!(
            ok(&[
                &["h", &h],
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &hex]
            ]),
            Ok(Some(LifecycleGuard::State {
                archive: true,
                scope: LifecycleScope::Subtree,
                ..
            }))
        ));
        assert!(matches!(
            ok(&[
                &["h", &h],
                &["archived", "false"],
                &["scope", "target"],
                &["expect", &hex]
            ]),
            Ok(Some(LifecycleGuard::State {
                archive: false,
                scope: LifecycleScope::Target,
                ..
            }))
        ));
        assert_eq!(
            ok(&[&["h", &h], &["lifecycle", "fence"], &["expect-target", "-"]]),
            Ok(Some(LifecycleGuard::Fence {
                expect_target: None
            }))
        );
        assert!(matches!(
            ok(&[
                &["h", &h],
                &["lifecycle", "fence"],
                &["expect-target", &token]
            ]),
            Ok(Some(LifecycleGuard::Fence {
                expect_target: Some(_)
            }))
        ));
        let upper = hex.to_uppercase();
        let other = Uuid::new_v4().to_string();
        let h_upper = h.to_uppercase();
        let state = |target: &[&str]| -> Vec<Vec<String>> {
            [
                target,
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &hex],
            ]
            .iter()
            .map(|t| t.iter().map(|v| v.to_string()).collect())
            .collect()
        };
        // The target is one canonical `h` tag of exactly two elements.
        for tags in [
            state(&["h", &h, "x"]),
            state(&["h", &h_upper]),
            state(&["h", "not-a-uuid"]),
            [state(&["h", &h]), vec![vec!["h".into(), other.clone()]]].concat(),
            state(&["d", &h]),
        ] {
            let tags: Vec<Vec<&str>> = tags
                .iter()
                .map(|t| t.iter().map(String::as_str).collect())
                .collect();
            let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
            assert!(ok(&tags).is_err(), "{tags:?}");
        }
        let refused: &[&[&[&str]]] = &[
            // Archive is always subtree.
            &[
                &["archived", "true"],
                &["scope", "target"],
                &["expect", &hex],
            ],
            // An archived tag of any other arity.
            &[
                &["archived", "true", "x"],
                &["scope", "subtree"],
                &["expect", &hex],
            ],
            &[&["archived"], &["scope", "subtree"], &["expect", &hex]],
            // Missing or unknown parts.
            &[&["archived", "true"], &["expect", &hex]],
            &[&["archived", "true"], &["scope", "subtree"]],
            &[&["archived", "true"], &["scope", "all"], &["expect", &hex]],
            &[&["scope", "subtree"], &["expect", &hex]],
            &[
                &["archived", "maybe"],
                &["scope", "subtree"],
                &["expect", &hex],
            ],
            &[
                &["archived", "true"],
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &hex],
            ],
            &[
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &upper],
            ],
            &[
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", "abc"],
            ],
            &[
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &hex],
                &["expect", &hex],
            ],
            &[
                &["archived", "true"],
                &["scope", "subtree", "x"],
                &["expect", &hex],
            ],
            // Settings changes do not ride on a guarded request.
            &[
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &hex],
                &["name", "x"],
            ],
            &[
                &["lifecycle", "fence"],
                &["expect-target", "-"],
                &["ttl", "60"],
            ],
            // Fence shape.
            &[&["lifecycle", "fence"]],
            &[&["lifecycle", "settle"], &["expect-target", "-"]],
            &[
                &["lifecycle", "fence"],
                &["expect-target", "-"],
                &["archived", "true"],
            ],
            &[
                &["lifecycle", "fence"],
                &["expect-target", "-"],
                &["expect", &hex],
            ],
            &[
                &["lifecycle", "fence"],
                &["expect-target", &token.to_uppercase()],
            ],
            &[
                &["archived", "true"],
                &["scope", "subtree"],
                &["expect", &hex],
                &["expect-target", "-"],
            ],
            &[&["expect-target", "-"]],
        ];
        let target = ["h", h.as_str()];
        for tags in refused {
            let with_h: Vec<&[&str]> = std::iter::once(&target[..])
                .chain(tags.iter().copied())
                .collect();
            assert!(ok(&with_h).is_err(), "{tags:?}");
        }
    }

    fn revision(
        op: &str,
        channel: Uuid,
        parent_tag: Option<&str>,
        content: &str,
    ) -> (Event, ArtifactEnvelope) {
        let mut tags = vec![
            vec!["ar".to_string(), "1".into()],
            vec!["d".into(), task_d(channel).to_string()],
            vec!["h".into(), channel.to_string()],
            vec!["type".into(), "nexus.task".into()],
            vec!["op".into(), op.into()],
        ];
        if let Some(value) = parent_tag {
            tags.push(vec!["parent".into(), value.into()]);
        }
        if op != "delete" {
            tags.push(vec!["title".into(), "Task".into()]);
        }
        if op != "create" {
            tags.push(vec!["prev".into(), "00".repeat(32)]);
        }
        let tags: Vec<Vec<&str>> = tags
            .iter()
            .map(|t| t.iter().map(String::as_str).collect())
            .collect();
        let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
        let event = event(45010, &tags, content);
        let env = crate::artifact::validate(&event).unwrap();
        (event, env)
    }

    #[test]
    fn task_profile_binds_d_content_and_parent() {
        let channel = Uuid::new_v4();
        let parent = Uuid::new_v4();
        let root = format!(r#"{{"v":0,"channelId":"{channel}","title":"Task"}}"#);
        let child = format!(r#"{{"v":0,"channelId":"{channel}","parentChannelId":"{parent}"}}"#);
        let (e, env) = revision("create", channel, None, &root);
        assert_eq!(
            task_edge(&e, &env),
            Ok(Some(TaskEdge {
                channel,
                parent: None
            }))
        );
        let (e, env) = revision("update", channel, Some(&parent.to_string()), &child);
        assert_eq!(
            task_edge(&e, &env),
            Ok(Some(TaskEdge {
                channel,
                parent: Some(parent)
            }))
        );
        let (e, env) = revision("delete", channel, None, "");
        assert_eq!(
            task_edge(&e, &env),
            Ok(Some(TaskEdge {
                channel,
                parent: None
            }))
        );

        let other = Uuid::new_v4();
        let refused = [
            // Tag without content parent, and the reverse.
            revision("update", channel, Some(&parent.to_string()), &root),
            revision("update", channel, None, &child),
            // Disagreeing parents.
            revision("update", channel, Some(&other.to_string()), &child),
            // Content for another channel, wrong version, not JSON.
            revision(
                "update",
                channel,
                None,
                &format!(r#"{{"v":0,"channelId":"{other}"}}"#),
            ),
            revision(
                "update",
                channel,
                None,
                &format!(r#"{{"v":1,"channelId":"{channel}"}}"#),
            ),
            revision(
                "update",
                channel,
                None,
                &format!(r#"{{"v":0.0,"channelId":"{channel}"}}"#),
            ),
            revision("update", channel, None, "not json"),
            // Non-canonical parent.
            revision(
                "update",
                channel,
                Some(&parent.to_string().to_uppercase()),
                &child,
            ),
            // An explicit null is not a root, with or without a parent tag.
            revision(
                "update",
                channel,
                None,
                &format!(r#"{{"v":0,"channelId":"{channel}","parentChannelId":null}}"#),
            ),
            revision(
                "update",
                channel,
                Some(&parent.to_string()),
                &format!(r#"{{"v":0,"channelId":"{channel}","parentChannelId":null}}"#),
            ),
            // A repeated hierarchy key is refused, whichever value is last.
            revision(
                "update",
                channel,
                Some(&parent.to_string()),
                &format!(
                    r#"{{"v":0,"channelId":"{channel}","parentChannelId":"{other}","parentChannelId":"{parent}"}}"#
                ),
            ),
            revision(
                "update",
                channel,
                None,
                &format!(r#"{{"v":0,"channelId":"{other}","channelId":"{channel}"}}"#),
            ),
            revision(
                "update",
                channel,
                None,
                &format!(r#"{{"v":1,"v":0,"channelId":"{channel}"}}"#),
            ),
            // Self-parent.
            revision(
                "update",
                channel,
                Some(&channel.to_string()),
                &format!(r#"{{"v":0,"channelId":"{channel}","parentChannelId":"{channel}"}}"#),
            ),
        ];
        for (e, env) in refused {
            assert!(task_edge(&e, &env).is_err(), "{}", e.content);
        }

        // A d that is not derived from h is refused; other types are ignored.
        let wrong_d = event(
            45010,
            &[
                &["ar", "1"],
                &["d", &Uuid::new_v4().to_string()],
                &["h", &channel.to_string()],
                &["type", "nexus.task"],
                &["op", "create"],
                &["title", "Task"],
            ],
            &root,
        );
        let env = crate::artifact::validate(&wrong_d).unwrap();
        assert!(task_edge(&wrong_d, &env).is_err());
        let other_type = event(
            45010,
            &[
                &["ar", "1"],
                &["d", &Uuid::new_v4().to_string()],
                &["h", &channel.to_string()],
                &["type", "nexus.note"],
                &["op", "create"],
                &["title", "Note"],
                &["parent", "anything"],
            ],
            "opaque",
        );
        let env = crate::artifact::validate(&other_type).unwrap();
        assert_eq!(task_edge(&other_type, &env), Ok(None));
    }
}
