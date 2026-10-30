//! `buzz community theme` — read and set your community theme preference.
//!
//! The theme is a per-user, per-community appearance preference, not a
//! community-wide setting: kind 30078 with `d=community-theme`, NIP-44
//! encrypted to the author's own key, exactly as Buzz Desktop reads and
//! writes it (`desktop/src/shared/theme/communityThemeSync.ts`). Any member
//! may write their own; the relay applies no admin check.

use nostr::nips::nip44;
use nostr::{EventBuilder, Keys, Timestamp};
use serde_json::Value;

use crate::client::{normalize_write_response, BuzzClient};
use crate::commands::parse_write_response;
use crate::error::CliError;

const D_TAG: &str = buzz_sdk::COMMUNITY_THEME_D_TAG;
const HEAD_QUERY_LIMIT: u32 = 5;

/// Mirrors `SYNTAX_THEMES` in `desktop/src/shared/theme/theme-loader.ts`.
pub(crate) const THEMES: &[&str] = &[
    "buzz",
    "buzz-dark",
    "andromeeda",
    "aurora-x",
    "ayu-dark",
    "catppuccin-frappe",
    "catppuccin-latte",
    "catppuccin-macchiato",
    "catppuccin-mocha",
    "dark-plus",
    "dracula",
    "dracula-soft",
    "everforest-dark",
    "everforest-light",
    "github-dark",
    "github-dark-default",
    "github-dark-dimmed",
    "github-dark-high-contrast",
    "github-light",
    "github-light-default",
    "github-light-high-contrast",
    "gruvbox-dark-hard",
    "gruvbox-dark-medium",
    "gruvbox-dark-soft",
    "gruvbox-light-hard",
    "gruvbox-light-medium",
    "gruvbox-light-soft",
    "houston",
    "kanagawa-dragon",
    "kanagawa-lotus",
    "kanagawa-wave",
    "laserwave",
    "light-plus",
    "material-theme",
    "material-theme-darker",
    "material-theme-lighter",
    "material-theme-ocean",
    "material-theme-palenight",
    "min-dark",
    "min-light",
    "monokai",
    "night-owl",
    "nord",
    "one-dark-pro",
    "one-light",
    "plastic",
    "poimandres",
    "red",
    "rose-pine",
    "rose-pine-dawn",
    "rose-pine-moon",
    "slack-dark",
    "slack-ochin",
    "snazzy-light",
    "solarized-dark",
    "solarized-light",
    "synthwave-84",
    "tokyo-night",
    "vesper",
    "vitesse-black",
    "vitesse-dark",
    "vitesse-light",
];

/// Mirrors the `value` of each `ACCENT_COLORS` entry in
/// `desktop/src/shared/theme/ThemeProvider.tsx`.
pub(crate) const ACCENTS: &[&str] = &[
    "neutral", "#3b82f6", "#06b6d4", "#22c55e", "#f97316", "#ef4444", "#ec4899", "#c0a2f1",
    "#a855f7", "#6366f1", "#d7d72e",
];

/// A valid stored theme, as Desktop's `parseCommunityThemePreference` accepts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Theme {
    theme: String,
    accent: String,
    follow_system: bool,
}

impl Theme {
    /// Desktop's `DEFAULT_COMMUNITY_THEME`, the base when no theme is stored.
    fn default_theme() -> Self {
        Self {
            theme: "buzz".into(),
            accent: "#3b82f6".into(),
            follow_system: true,
        }
    }

    fn to_json(&self) -> Value {
        serde_json::json!({
            "version": 1,
            "theme": self.theme,
            "accent": self.accent,
            "followSystem": self.follow_system,
        })
    }

    fn from_json(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.get("version")?.as_i64()? != 1 {
            return None;
        }
        let theme = object.get("theme")?.as_str()?;
        let accent = object.get("accent")?.as_str()?;
        if !THEMES.contains(&theme) || !ACCENTS.contains(&accent) {
            return None;
        }
        Some(Self {
            theme: theme.into(),
            accent: accent.into(),
            follow_system: object.get("followSystem")?.as_bool()?,
        })
    }
}

/// The fields a `set` call changes. `None` leaves the stored value alone.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ThemePatch {
    theme: Option<String>,
    accent: Option<String>,
    follow_system: Option<bool>,
}

impl ThemePatch {
    /// Validate raw flag values against the closed sets Desktop accepts.
    pub(crate) fn parse(
        theme: Option<String>,
        accent: Option<String>,
        follow_system: Option<bool>,
    ) -> Result<Self, CliError> {
        if theme.is_none() && accent.is_none() && follow_system.is_none() {
            return Err(CliError::Usage(
                "pass at least one of --theme, --accent, --follow-system".into(),
            ));
        }
        if let Some(value) = &theme {
            check_member("--theme", value, THEMES)?;
        }
        if let Some(value) = &accent {
            check_member("--accent", value, ACCENTS)?;
        }
        Ok(Self {
            theme,
            accent,
            follow_system,
        })
    }

    fn is_complete(&self) -> bool {
        self.theme.is_some() && self.accent.is_some() && self.follow_system.is_some()
    }

    fn apply(&self, base: Theme) -> Theme {
        Theme {
            theme: self.theme.clone().unwrap_or(base.theme),
            accent: self.accent.clone().unwrap_or(base.accent),
            follow_system: self.follow_system.unwrap_or(base.follow_system),
        }
    }
}

fn check_member(flag: &str, value: &str, allowed: &[&str]) -> Result<(), CliError> {
    if allowed.contains(&value) {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "invalid value {value:?} for {flag}; valid values: {}",
        allowed.join(", ")
    )))
}

/// The newest stored theme event. `theme` is `None` when it does not decrypt
/// or validate.
#[derive(Debug)]
pub(crate) struct Head {
    id: String,
    created_at: u64,
    theme: Option<Theme>,
}

fn newer(candidate: (u64, &str), current: (u64, &str)) -> bool {
    candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
}

/// Pick the winning event the way Desktop does (newest `created_at`, lowest id
/// on a tie) and decrypt it. Events from any other author are an error, never
/// skipped.
pub(crate) fn decode_head(keys: &Keys, events: &[Value]) -> Result<Option<Head>, CliError> {
    let me = keys.public_key().to_hex();
    let mut best: Option<(&Value, u64, &str)> = None;
    for event in events {
        let field = |name: &str| event.get(name).and_then(Value::as_str);
        if field("pubkey") != Some(me.as_str()) {
            return Err(CliError::Other(
                "relay returned a community theme event from another author".into(),
            ));
        }
        let (Some(id), Some(created_at)) =
            (field("id"), event.get("created_at").and_then(Value::as_u64))
        else {
            return Err(CliError::Other(
                "relay returned a malformed community theme event".into(),
            ));
        };
        if best.is_none_or(|(_, at, best_id)| newer((created_at, id), (at, best_id))) {
            best = Some((event, created_at, id));
        }
    }
    let Some((event, created_at, id)) = best else {
        return Ok(None);
    };
    let theme = event
        .get("content")
        .and_then(Value::as_str)
        .and_then(|content| nip44::decrypt(keys.secret_key(), &keys.public_key(), content).ok())
        .and_then(|plaintext| serde_json::from_str::<Value>(&plaintext).ok())
        .and_then(|value| Theme::from_json(&value));
    Ok(Some(Head {
        id: id.into(),
        created_at,
        theme,
    }))
}

/// Build the replacement event. Its `created_at` is at least one second after
/// the head so NIP-33 resolution keeps the new event.
pub(crate) fn build_event(
    keys: &Keys,
    theme: &Theme,
    head_created_at: Option<u64>,
    now: u64,
) -> Result<EventBuilder, CliError> {
    let ciphertext = nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        theme.to_json().to_string(),
        nip44::Version::V2,
    )
    .map_err(|e| CliError::Other(format!("community theme encryption failed: {e}")))?;
    let created_at = head_created_at.map_or(now, |head| now.max(head.saturating_add(1)));
    buzz_sdk::build_community_theme(&ciphertext, created_at)
        .map_err(|e| CliError::Other(format!("build_community_theme failed: {e}")))
}

/// Decide what `set` publishes: `None` when the patch changes nothing.
pub(crate) fn plan_set(head: Option<&Head>, patch: &ThemePatch) -> Result<Option<Theme>, CliError> {
    let base = match head {
        None => Theme::default_theme(),
        Some(Head {
            theme: Some(theme), ..
        }) => theme.clone(),
        Some(_) if patch.is_complete() => Theme::default_theme(),
        Some(_) => {
            return Err(CliError::Other(
                "the stored community theme is not readable; pass --theme, --accent and \
                 --follow-system together to replace it"
                    .into(),
            ))
        }
    };
    let next = patch.apply(base);
    let unchanged = head.and_then(|h| h.theme.as_ref()) == Some(&next);
    Ok((!unchanged).then_some(next))
}

async fn fetch_head(client: &BuzzClient) -> Result<Option<Head>, CliError> {
    let filter = serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_READ_STATE],
        "authors": [client.keys().public_key().to_hex()],
        "#d": [D_TAG],
        "limit": HEAD_QUERY_LIMIT,
    });
    let raw = client.query(&filter).await?;
    let events: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse community theme query: {e}")))?;
    decode_head(client.keys(), &events)
}

async fn cmd_get(client: &BuzzClient) -> Result<(), CliError> {
    let output = match fetch_head(client).await? {
        None => Value::Null,
        Some(Head {
            theme: Some(theme), ..
        }) => theme.to_json(),
        Some(head) => {
            return Err(CliError::Other(format!(
                "the stored community theme (event {}) is not readable by Buzz Desktop either",
                head.id
            )))
        }
    };
    println!("{output}");
    Ok(())
}

async fn cmd_set(client: &BuzzClient, patch: ThemePatch) -> Result<(), CliError> {
    let head = fetch_head(client).await?;
    let Some(next) = plan_set(head.as_ref(), &patch)? else {
        let event_id = head.map(|h| h.id).unwrap_or_default();
        println!(
            "{}",
            serde_json::json!({"event_id": event_id, "accepted": true, "message": "unchanged"})
        );
        return Ok(());
    };
    let builder = build_event(
        client.keys(),
        &next,
        head.as_ref().map(|h| h.created_at),
        Timestamp::now().as_secs(),
    )?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    parse_write_response(
        &resp,
        "community theme was superseded by a newer write; re-run to retry",
    )?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(cmd: crate::CommunityCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::{CommunityCmd, CommunityThemeCmd};
    match cmd {
        CommunityCmd::Theme(CommunityThemeCmd::Get) => cmd_get(client).await,
        CommunityCmd::Theme(CommunityThemeCmd::Set {
            theme,
            accent,
            follow_system,
        }) => cmd_set(client, ThemePatch::parse(theme, accent, follow_system)?).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::generate()
    }

    fn quoted_strings(block: &str) -> Vec<String> {
        block
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    fn desktop_source(relative: &str) -> String {
        let path = format!(
            "{}/../../desktop/src/shared/theme/{relative}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    fn signed_json(keys: &Keys, theme: &Theme, head_created_at: Option<u64>, now: u64) -> Value {
        let builder = build_event(keys, theme, head_created_at, now).expect("build");
        let event = builder.sign_with_keys(keys).expect("sign");
        serde_json::to_value(&event).expect("event json")
    }

    fn stored(theme: &str, accent: &str, follow_system: bool) -> Theme {
        Theme {
            theme: theme.into(),
            accent: accent.into(),
            follow_system,
        }
    }

    #[test]
    fn enums_match_desktop_sources() {
        let loader = desktop_source("theme-loader.ts");
        let themes = loader
            .split("export const SYNTAX_THEMES = [")
            .nth(1)
            .and_then(|rest| rest.split("] as const").next())
            .expect("SYNTAX_THEMES block");
        assert_eq!(quoted_strings(themes), THEMES);

        let provider = desktop_source("ThemeProvider.tsx");
        let accents = provider
            .split("export const ACCENT_COLORS = [")
            .nth(1)
            .and_then(|rest| rest.split("] as const").next())
            .expect("ACCENT_COLORS block")
            .lines()
            .filter_map(|line| line.split("value: ").nth(1))
            .map(|value| {
                value
                    .trim_end_matches([' ', '}', ','])
                    .trim_matches('"')
                    .to_string()
            })
            .map(|value| {
                if value == "NEUTRAL_ACCENT" {
                    "neutral".into()
                } else {
                    value
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(accents, ACCENTS);
    }

    #[test]
    fn patch_accepts_every_desktop_enum_value() {
        for theme in THEMES {
            ThemePatch::parse(Some((*theme).into()), None, None).expect(theme);
        }
        for accent in ACCENTS {
            ThemePatch::parse(None, Some((*accent).into()), None).expect(accent);
        }
        ThemePatch::parse(None, None, Some(false)).expect("follow-system");
    }

    #[test]
    fn patch_rejects_values_outside_the_closed_sets_and_lists_valid_ones() {
        for (theme, accent, needle) in [
            (Some("not-a-theme"), None, "nord"),
            (Some("Nord"), None, "nord"),
            (None, Some("#3B82F6"), "#3b82f6"),
            (None, Some("blue"), "neutral"),
            (None, Some(""), "#22c55e"),
        ] {
            let err = ThemePatch::parse(theme.map(Into::into), accent.map(Into::into), None)
                .expect_err("must reject");
            assert!(matches!(err, CliError::Usage(_)), "usage error: {err}");
            assert!(
                err.to_string().contains(needle),
                "lists valid values: {err}"
            );
        }
        let err = ThemePatch::parse(None, None, None).expect_err("empty patch");
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn cli_rejects_non_boolean_follow_system() {
        assert!(<crate::Cli as clap::Parser>::try_parse_from([
            "buzz",
            "community",
            "theme",
            "set",
            "--follow-system",
            "maybe"
        ])
        .is_err());
        assert!(<crate::Cli as clap::Parser>::try_parse_from([
            "buzz",
            "community",
            "theme",
            "set",
            "--follow-system",
            "false"
        ])
        .is_ok());
        assert!(<crate::Cli as clap::Parser>::try_parse_from([
            "buzz",
            "community",
            "theme",
            "get"
        ])
        .is_ok());
    }

    #[test]
    fn set_changes_only_the_given_fields_through_the_wire_format() {
        let keys = keys();
        let head_theme = stored("nord", "#22c55e", false);
        let head = decode_head(&keys, &[signed_json(&keys, &head_theme, None, 1_000)])
            .expect("decode")
            .expect("head");
        assert_eq!(head.theme.as_ref(), Some(&head_theme));

        let patch = ThemePatch::parse(None, Some("#ef4444".into()), None).unwrap();
        let next = plan_set(Some(&head), &patch)
            .expect("plan")
            .expect("changed");
        assert_eq!(next, stored("nord", "#ef4444", false));

        let republished = decode_head(
            &keys,
            &[signed_json(&keys, &next, Some(head.created_at), 2_000)],
        )
        .unwrap()
        .unwrap();
        assert_eq!(republished.theme, Some(next));
    }

    #[test]
    fn set_without_a_stored_theme_starts_from_desktop_defaults() {
        let patch = ThemePatch::parse(Some("dracula".into()), None, None).unwrap();
        let next = plan_set(None, &patch).unwrap().unwrap();
        assert_eq!(next, stored("dracula", "#3b82f6", true));
    }

    #[test]
    fn set_that_changes_nothing_publishes_nothing() {
        let keys = keys();
        let theme = stored("nord", "neutral", true);
        let head = decode_head(&keys, &[signed_json(&keys, &theme, None, 1_000)])
            .unwrap()
            .unwrap();
        let patch = ThemePatch::parse(Some("nord".into()), None, Some(true)).unwrap();
        assert_eq!(plan_set(Some(&head), &patch).unwrap(), None);
    }

    #[test]
    fn replacement_is_strictly_newer_than_the_head() {
        let keys = keys();
        let theme = stored("buzz", "#3b82f6", true);
        for (head_at, now, expected) in [
            (5_000, 4_000, 5_001),
            (5_000, 5_000, 5_001),
            (5_000, 6_000, 6_000),
        ] {
            let event = signed_json(&keys, &theme, Some(head_at), now);
            assert_eq!(event["created_at"].as_u64(), Some(expected));
        }
        assert_eq!(
            signed_json(&keys, &theme, None, 7_000)["created_at"].as_u64(),
            Some(7_000)
        );
    }

    #[test]
    fn unreadable_head_blocks_partial_set_but_allows_complete_replacement() {
        let keys = keys();
        let mut event = signed_json(&keys, &stored("buzz", "#3b82f6", true), None, 1_000);
        event["content"] = Value::String("not-ciphertext".into());
        let head = decode_head(&keys, &[event]).unwrap().unwrap();
        assert!(head.theme.is_none());

        let partial = ThemePatch::parse(Some("nord".into()), None, None).unwrap();
        assert!(matches!(
            plan_set(Some(&head), &partial),
            Err(CliError::Other(_))
        ));

        let complete =
            ThemePatch::parse(Some("nord".into()), Some("neutral".into()), Some(true)).unwrap();
        assert_eq!(
            plan_set(Some(&head), &complete).unwrap(),
            Some(stored("nord", "neutral", true))
        );
    }

    #[test]
    fn head_with_a_value_desktop_would_reject_is_unreadable() {
        let keys = keys();
        for doc in [
            serde_json::json!({"version": 2, "theme": "buzz", "accent": "#3b82f6", "followSystem": true}),
            serde_json::json!({"version": 1, "theme": "bogus", "accent": "#3b82f6", "followSystem": true}),
            serde_json::json!({"version": 1, "theme": "buzz", "accent": "#000000", "followSystem": true}),
            serde_json::json!({"version": 1, "theme": "buzz", "accent": "#3b82f6", "followSystem": "yes"}),
        ] {
            let ciphertext = nip44::encrypt(
                keys.secret_key(),
                &keys.public_key(),
                doc.to_string(),
                nip44::Version::V2,
            )
            .unwrap();
            let event = buzz_sdk::build_community_theme(&ciphertext, 10)
                .unwrap()
                .sign_with_keys(&keys)
                .unwrap();
            let head = decode_head(&keys, &[serde_json::to_value(&event).unwrap()])
                .unwrap()
                .unwrap();
            assert!(head.theme.is_none(), "{doc}");
        }
    }

    #[test]
    fn head_selection_matches_desktop_ordering_and_rejects_foreign_authors() {
        let keys = keys();
        let a = signed_json(&keys, &stored("nord", "#3b82f6", true), None, 100);
        let b = signed_json(&keys, &stored("dracula", "#3b82f6", true), None, 200);
        let head = decode_head(&keys, &[a.clone(), b.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(head.id, b["id"].as_str().unwrap());

        let mut tie_a = a.clone();
        let mut tie_b = b.clone();
        tie_a["created_at"] = 300.into();
        tie_b["created_at"] = 300.into();
        let lower = if tie_a["id"].as_str() < tie_b["id"].as_str() {
            &tie_a
        } else {
            &tie_b
        };
        let head = decode_head(&keys, &[tie_a.clone(), tie_b.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(head.id, lower["id"].as_str().unwrap());

        assert!(decode_head(&keys, &[]).unwrap().is_none());
        let stranger = Keys::generate();
        let foreign = signed_json(&stranger, &stored("nord", "#3b82f6", true), None, 100);
        assert!(matches!(
            decode_head(&keys, &[foreign]),
            Err(CliError::Other(_))
        ));
    }

    #[test]
    fn serialized_theme_is_the_exact_desktop_document() {
        assert_eq!(
            stored("nord", "neutral", false).to_json().to_string(),
            r##"{"accent":"neutral","followSystem":false,"theme":"nord","version":1}"##
        );
    }

    #[test]
    fn event_is_kind_30078_with_desktop_tags_and_self_encryption() {
        let keys = keys();
        let event = build_event(&keys, &stored("buzz", "#3b82f6", true), None, 42)
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(event.kind.as_u16(), 30078);
        let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
        assert_eq!(
            tags,
            vec![vec!["d", "community-theme"], vec!["t", "community-theme"]]
        );
        let plain = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content).unwrap();
        assert!(plain.contains("\"followSystem\":true"));
    }
}
