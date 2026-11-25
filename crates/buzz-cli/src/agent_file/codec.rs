//! Converts between an agent document and `AGENT.md` text.
//!
//! `render` writes the canonical form. `parse` reads any valid file, with
//! comments and keys in any order. For a document the format can express,
//! `parse(render(doc))` gives `doc` back, apart from surrounding whitespace in
//! prompts, which the format doesn't keep.

use std::collections::{BTreeMap, BTreeSet};

use nostr::nips::nip19::ToBech32;
use nostr::PublicKey;
use serde_json::Value;

use super::document::{
    AgentDocument, AgentRef, Attention, AttentionValue, ChannelMembership, Classifier, EventWatch,
    Interest, Profile, Question, TimerWatch, DOCUMENT_VERSION,
};
use super::format::{
    BuzzBlock, ChannelEntry, Event, FrontMatter, FullQuestion, Identity, InterestEntry, Max,
    QuestionSpec, Timer, UnboundedWord, Wake, KIND, VERSION,
};

/// A file that can't be read, or an agent the format can't express.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CodecError(pub String);

fn fail<T>(message: impl Into<String>) -> Result<T, CodecError> {
    Err(CodecError(message.into()))
}

const HEADING: &str = "## When ";
const BUZZ_SOURCE: &str = "buzz";
const INTEREST: &str = "interest/";
const WATCH: &str = "watch/";
/// The settings that have their own top-level fields.
const TOP_LEVEL_SETTINGS: [&str; 2] = ["model", "effort"];
/// Fields of the shared format that a Buzz agent can't run yet.
const UNSUPPORTED_TOP: [&str; 3] = ["skills", "mcps", "limits"];
const UNSUPPORTED_WAKE: [&str; 2] = ["schedules", "session"];
const UNSUPPORTED_IDENTITY: [&str; 2] = ["run_as", "owner"];
const UNITS: [(char, u64); 4] = [('d', 86_400), ('h', 3_600), ('m', 60), ('s', 1)];

/// An `AGENT.md` file, read.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFile {
    /// The file's `name`, which must match its directory.
    pub name: String,
    pub document: AgentDocument,
}

/// Writes `doc` as a canonical `AGENT.md` named `name`.
pub fn render(doc: &AgentDocument, name: &str) -> Result<String, CodecError> {
    if doc.version != DOCUMENT_VERSION {
        return fail(format!(
            "The app sent agent document version {}; this buzz reads version {DOCUMENT_VERSION}. Update buzz.",
            doc.version
        ));
    }
    check_name(name)?;
    let (wake, interests, sections) = render_attention(&doc.attention)?;
    if has_heading(&doc.instructions) {
        return fail(format!(
            "The persona prompt has a \"{}\" heading naming wake ids, or an unclosed code fence, which the file would read as a section",
            HEADING.trim_end()
        ));
    }

    let mut settings = doc.settings.clone();
    let model = take_text(&mut settings, "model")?;
    let effort = take_text(&mut settings, "effort")?;
    let settings = settings
        .into_iter()
        .map(|(key, value)| Ok((key, json_to_yaml(&value)?)))
        .collect::<Result<BTreeMap<_, _>, CodecError>>()?;
    let mut channels: Vec<ChannelEntry> = doc
        .channels
        .iter()
        .map(|c| ChannelEntry {
            channel: c.channel.clone(),
            role: c.role.clone(),
        })
        .collect();
    channels.sort_by(|a, b| a.channel.cmp(&b.channel));

    let identity = match &doc.agent {
        Some(agent) => Some(Identity {
            buzz: Some(npub(&agent.pubkey)?),
        }),
        None => None,
    };
    let front = FrontMatter {
        kind: KIND.to_owned(),
        version: VERSION,
        name: name.to_owned(),
        display_name: Some(doc.profile.name.clone()),
        description: doc.profile.about.clone(),
        enabled: true,
        identity,
        avatar: doc.profile.picture.clone(),
        model,
        effort,
        buzz: Some(BuzzBlock {
            community: doc.agent.as_ref().and_then(|a| a.community.clone()),
            runtime: doc.agent_type.clone(),
            settings,
            channels,
            interests,
        }),
        wake,
    };
    let yaml = serde_yaml::to_string(&front)
        .map_err(|e| CodecError(format!("Could not write the front matter: {e}")))?;

    let mut out = format!("---\n{yaml}---\n");
    let mut body = String::new();
    let persona = tidy(&doc.instructions);
    if !persona.is_empty() {
        body.push_str(persona);
        body.push('\n');
    }
    for (heading, text) in sections {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(HEADING);
        body.push_str(&heading);
        body.push_str("\n\n");
        body.push_str(tidy(&text));
        body.push('\n');
    }
    if !body.is_empty() {
        out.push('\n');
        out.push_str(&body);
    }
    Ok(out)
}

/// A `## When` heading and the section's text.
type Section = (String, String);

/// Turns attention objects into wakes, Interest entries and `## When` sections.
fn render_attention(
    attention: &Attention,
) -> Result<(Wake, Vec<InterestEntry>, Vec<Section>), CodecError> {
    let mut interests: BTreeMap<&str, &Interest> = BTreeMap::new();
    let mut feeds: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut events = Vec::new();
    let mut timers = Vec::new();
    for (slug, value) in &attention.objects {
        match value {
            AttentionValue::Interest(interest) => {
                let Some(id) = slug.strip_prefix(INTEREST) else {
                    return fail(format!("{slug} is an Interest outside interest/"));
                };
                if has_heading(&interest.instructions) {
                    return fail(format!(
                        "Interest {id}'s instructions have a \"{}\" heading naming wake ids, or an unclosed code fence",
                        HEADING.trim_end()
                    ));
                }
                interests.insert(id, interest);
            }
            AttentionValue::Source(_) => {
                return fail(format!(
                    "{slug} is a plugin source watch, which AGENT.md can't express yet"
                ));
            }
            AttentionValue::Event(_) | AttentionValue::Timer(_) => {
                let Some(id) = slug.strip_prefix(WATCH) else {
                    return fail(format!("{slug} is a watch outside watch/"));
                };
                check_wake_id(id).map_err(|e| CodecError(format!("{slug}: {}", e.0)))?;
                match value {
                    AttentionValue::Event(watch) => {
                        feeds.entry(&watch.interest_id).or_default().push(id);
                        events.push((id, watch));
                    }
                    AttentionValue::Timer(timer) => {
                        feeds.entry(&timer.interest_id).or_default().push(id);
                        timers.push((id, timer));
                    }
                    _ => {}
                }
            }
        }
    }
    for (interest, wakes) in &feeds {
        if !interests.contains_key(interest) {
            return fail(format!(
                "watch/{} feeds Interest {interest}, which doesn't exist",
                wakes[0]
            ));
        }
    }

    // A section's heading is its wakes, sorted; the Interest id defaults to
    // the first of them, so `interest` is written only when it differs.
    let mut entries = Vec::new();
    let mut sections = Vec::new();
    for (id, interest) in &interests {
        let wakes = feeds.get(id).map(Vec::as_slice).unwrap_or_default();
        let mut wakes = wakes.to_vec();
        wakes.sort_unstable();
        let has_details =
            interest.title.is_some() || interest.summary.is_some() || interest.paused.is_some();
        if wakes.is_empty() || has_details {
            entries.push(InterestEntry {
                id: (*id).to_owned(),
                title: interest.title.clone(),
                summary: interest.summary.clone(),
                paused: interest.paused,
                instructions: wakes
                    .is_empty()
                    .then(|| tidy(&interest.instructions).to_owned()),
            });
        }
        if !wakes.is_empty() {
            sections.push((wakes.join(", "), interest.instructions.clone()));
        }
    }
    sections.sort();
    let named = |interest: &str| -> Option<String> {
        let mut wakes = feeds.get(interest)?.clone();
        wakes.sort_unstable();
        (wakes.first().copied() != Some(interest)).then(|| interest.to_owned())
    };

    let mut wake = Wake::default();
    for (id, timer) in timers {
        wake.timers.push(Timer {
            id: id.to_owned(),
            enabled: timer.enabled,
            every: every(timer.interval_secs)?,
            max: match timer.max_occurrences {
                Some(count) => Max::Count(count),
                None => Max::Unbounded(UnboundedWord::Unbounded),
            },
            interest: named(&timer.interest_id),
            prompt: timer.prompt.clone(),
            expires_at: timer.expires_at,
            channel: timer.channel.clone(),
            root: timer.root.clone(),
        });
    }
    for (id, watch) in events {
        wake.events.push(Event {
            id: id.to_owned(),
            enabled: watch.enabled,
            source: BUZZ_SOURCE.to_owned(),
            interest: named(&watch.interest_id),
            name: watch.name.clone(),
            channels: watch.channels.clone(),
            kinds: watch.kinds.clone(),
            authors: watch.authors.clone(),
            exclude_authors: watch.exclude_authors.clone(),
            tags: watch.tags.clone(),
            is_reply: watch.is_reply,
            classifier: watch.classifier.as_ref().map(|c| {
                c.questions
                    .iter()
                    .map(|(name, q)| (name.clone(), question_spec(q)))
                    .collect()
            }),
            channel: watch.channel.clone(),
            root: watch.root.clone(),
        });
    }
    Ok((wake, entries, sections))
}

fn question_spec(q: &Question) -> QuestionSpec {
    if q.if_true.is_none() && q.if_false.is_none() && q.guidance.is_none() && q.threshold.is_none()
    {
        return QuestionSpec::Text(q.question.clone());
    }
    QuestionSpec::Full(FullQuestion {
        question: q.question.clone(),
        if_true: q.if_true.clone(),
        if_false: q.if_false.clone(),
        guidance: q.guidance.clone(),
        threshold: q.threshold,
    })
}

/// Reads an `AGENT.md`.
pub fn parse(text: &str) -> Result<ParsedFile, CodecError> {
    let (yaml, body) = buzz_persona::persona::split_frontmatter(text).map_err(|_| {
        CodecError("AGENT.md must start with YAML front matter between --- lines".into())
    })?;
    let mut raw: serde_yaml::Value =
        serde_yaml::from_str(yaml).map_err(|e| CodecError(format!("Front matter: {e}")))?;
    refuse_unsupported(&raw)?;
    quote_answer_keys(&mut raw);
    let front: FrontMatter =
        serde_yaml::from_value(raw).map_err(|e| CodecError(format!("Front matter: {e}")))?;

    if front.kind != KIND {
        return fail(format!("kind must be {KIND}"));
    }
    if front.version != 2 && front.version != VERSION {
        return fail(format!("version must be 2 or {VERSION}"));
    }
    check_name(&front.name)?;
    if !front.enabled {
        return fail(
            "enabled: false isn't supported for Buzz agents yet. Remove the agent's wakes instead.",
        );
    }
    let Some(buzz) = front.buzz else {
        return fail("A Buzz agent needs a buzz block with its runtime");
    };

    let (persona, sections) = split_sections(body)?;
    let objects = parse_attention(&front.wake, &buzz.interests, sections)?;

    let mut settings = serde_json::Map::new();
    for (key, value) in &buzz.settings {
        if TOP_LEVEL_SETTINGS.contains(&key.as_str()) {
            return fail(format!("Set {key} at the top level, not in buzz.settings"));
        }
        settings.insert(key.clone(), yaml_to_json(value)?);
    }
    if let Some(model) = front.model {
        settings.insert("model".into(), Value::String(model));
    }
    if let Some(effort) = front.effort {
        settings.insert("effort".into(), Value::String(effort));
    }

    let mut channels = Vec::new();
    let mut seen = BTreeSet::new();
    for entry in buzz.channels {
        if !seen.insert(entry.channel.clone()) {
            return fail(format!("Channel {} is listed twice", entry.channel));
        }
        channels.push(ChannelMembership {
            channel: entry.channel,
            role: entry.role,
        });
    }
    channels.sort();

    let agent = match front.identity.and_then(|i| i.buzz) {
        Some(npub) => Some(AgentRef {
            pubkey: PublicKey::parse(&npub)
                .map_err(|_| CodecError(format!("identity.buzz is not an npub: {npub}")))?
                .to_hex(),
            community: buzz.community,
            owner: None,
        }),
        None if buzz.community.is_some() => {
            return fail("buzz.community needs identity.buzz");
        }
        None => None,
    };

    Ok(ParsedFile {
        document: AgentDocument {
            version: DOCUMENT_VERSION,
            agent,
            agent_type: buzz.runtime,
            profile: Profile {
                name: front.display_name.unwrap_or_else(|| front.name.clone()),
                about: front.description,
                picture: front.avatar,
            },
            instructions: persona,
            settings,
            attention: Attention { objects },
            channels,
        },
        name: front.name,
    })
}

/// Names the fields a Buzz agent can't run yet, so the error says why.
fn refuse_unsupported(raw: &serde_yaml::Value) -> Result<(), CodecError> {
    let unsupported = |path: &str| {
        fail(format!(
            "{path} isn't supported for Buzz agents yet. Remove it to import this file."
        ))
    };
    for key in UNSUPPORTED_TOP {
        if raw.get(key).is_some() {
            return unsupported(key);
        }
    }
    if let Some(wake) = raw.get("wake") {
        for key in UNSUPPORTED_WAKE {
            if wake.get(key).is_some() {
                return unsupported(&format!("wake.{key}"));
            }
        }
        if let Some(events) = wake.get("events").and_then(|e| e.as_sequence()) {
            for event in events {
                let source = event.get("source").and_then(|s| s.as_str());
                if source.is_some_and(|s| s != BUZZ_SOURCE) {
                    let id = event.get("id").and_then(|s| s.as_str()).unwrap_or("?");
                    return unsupported(&format!(
                        "wake.events[{id}] with a source other than buzz"
                    ));
                }
            }
        }
    }
    if let Some(identity) = raw.get("identity") {
        for key in UNSUPPORTED_IDENTITY {
            if identity.get(key).is_some() {
                return unsupported(&format!("identity.{key}"));
            }
        }
    }
    Ok(())
}

/// YAML reads an unquoted `true:` or `false:` key as a boolean. In a
/// classifier question they name the answers, so read them as text.
fn quote_answer_keys(raw: &mut serde_yaml::Value) {
    let Some(events) = raw
        .get_mut("wake")
        .and_then(|w| w.get_mut("events"))
        .and_then(|e| e.as_sequence_mut())
    else {
        return;
    };
    for event in events {
        let Some(questions) = event.get_mut("classifier").and_then(|c| c.as_mapping_mut()) else {
            continue;
        };
        for (_, question) in questions.iter_mut() {
            let Some(fields) = question.as_mapping_mut() else {
                continue;
            };
            for answer in [true, false] {
                if let Some(text) = fields.remove(serde_yaml::Value::Bool(answer)) {
                    fields.insert(serde_yaml::Value::String(answer.to_string()), text);
                }
            }
        }
    }
}

/// Splits the body into the persona prompt and `(heading, text)` sections.
/// The wake ids a `## When` line names, as written, or `None` when the line
/// is ordinary prompt text. As in the shared format: a heading with a comma
/// names every comma-separated id; without one it names a single wake id, so
/// `## When the build fails` stays prompt text.
fn section_heading(line: &str) -> Option<&str> {
    let heading = line.strip_prefix(HEADING)?.trim_end_matches([' ', '\t']);
    if heading.is_empty() || heading.starts_with(char::is_whitespace) {
        return None;
    }
    (heading.contains(',') || slug_like(heading, 40)).then_some(heading)
}

/// The fence a line opens or closes: up to three spaces, then three or more
/// backticks or tildes.
fn fence_marker(line: &str) -> Option<&str> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = rest.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let run = rest.len() - rest.trim_start_matches(ch).len();
    (run >= 3).then(|| &rest[..run])
}

/// Splits the body's lines into those outside and inside fenced code, the
/// way the shared format does: a `## When` heading in a fence is prompt text.
/// Returns each line with whether it is fenced, and whether a fence is left
/// open at the end.
fn fenced_lines(body: &str) -> (Vec<(&str, bool)>, bool) {
    let mut fence: Option<&str> = None;
    let mut lines = Vec::new();
    for line in body.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        let marker = fence_marker(bare);
        if let Some(open) = fence {
            lines.push((line, true));
            if let Some(close) = marker {
                let after = &bare.trim_start_matches(' ')[close.len()..];
                if close.starts_with(&open[..1])
                    && close.len() >= open.len()
                    && after.trim().is_empty()
                {
                    fence = None;
                }
            }
        } else {
            if let Some(open) = marker {
                let after = &bare.trim_start_matches(' ')[open.len()..];
                if !(open.starts_with('`') && after.contains('`')) {
                    fence = Some(open);
                }
            }
            lines.push((line, false));
        }
    }
    (lines, fence.is_some())
}

fn split_sections(body: &str) -> Result<(String, Vec<(String, String)>), CodecError> {
    let mut persona = String::new();
    let mut sections: Vec<(String, String)> = Vec::new();
    for (line, fenced) in fenced_lines(body).0 {
        let bare = line.trim_end_matches(['\n', '\r']);
        if let Some(heading) = (!fenced).then(|| section_heading(bare)).flatten() {
            sections.push((heading.to_owned(), String::new()));
        } else if let Some((_, text)) = sections.last_mut() {
            text.push_str(line);
        } else {
            persona.push_str(line);
        }
    }
    Ok((
        tidy(&persona).to_owned(),
        sections
            .into_iter()
            .map(|(h, t)| (h, tidy(&t).to_owned()))
            .collect(),
    ))
}

/// Builds attention objects from wakes, Interest entries and sections.
fn parse_attention(
    wake: &Wake,
    entries: &[InterestEntry],
    sections: Vec<(String, String)>,
) -> Result<BTreeMap<String, AttentionValue>, CodecError> {
    // Every wake, with the Interest it names, if any.
    let mut wakes: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    for (id, interest) in wake
        .timers
        .iter()
        .map(|t| (t.id.as_str(), t.interest.as_deref()))
        .chain(
            wake.events
                .iter()
                .map(|e| (e.id.as_str(), e.interest.as_deref())),
        )
    {
        check_wake_id(id)?;
        if wakes.insert(id, interest).is_some() {
            return fail(format!("Wake id {id} is used twice"));
        }
    }

    // Each section sets one Interest's instructions.
    let mut resolved: BTreeMap<&str, String> = BTreeMap::new();
    let mut instructions: BTreeMap<String, String> = BTreeMap::new();
    let mut placed = BTreeSet::new();
    for (heading, text) in &sections {
        let ids: Vec<&str> = heading.split(',').map(str::trim).collect();
        if ids.contains(&"") {
            return fail(format!("## When {heading} has an empty wake id"));
        }
        let mut interest: Option<String> = None;
        for id in &ids {
            let Some(named) = wakes.get(id) else {
                return fail(format!("## When {heading} names {id}, which isn't a wake"));
            };
            if !placed.insert(*id) {
                return fail(format!("Wake {id} is in two ## When sections"));
            }
            let this = named.unwrap_or(ids[0]).to_owned();
            match &interest {
                Some(other) if *other != this => {
                    return fail(format!(
                        "## When {heading} names wakes that feed different Interests ({other} and {this})"
                    ));
                }
                _ => interest = Some(this),
            }
        }
        let interest = interest.unwrap_or_default();
        if text.is_empty() {
            return fail(format!("## When {heading} is empty"));
        }
        if instructions
            .insert(interest.clone(), text.clone())
            .is_some()
        {
            return fail(format!(
                "Interest {interest} has two ## When sections. Put its wakes in one heading."
            ));
        }
        for id in ids {
            resolved.insert(id, interest.clone());
        }
    }
    // A wake outside every section feeds the Interest it names, or its own.
    for (id, named) in &wakes {
        if !resolved.contains_key(id) {
            resolved.insert(id, named.unwrap_or(id).to_owned());
        }
    }

    let mut objects = BTreeMap::new();
    let mut listed = BTreeSet::new();
    for entry in entries {
        if !listed.insert(entry.id.as_str()) {
            return fail(format!("buzz.interests lists {} twice", entry.id));
        }
        let text = match (&entry.instructions, instructions.remove(&entry.id)) {
            (Some(_), Some(_)) => {
                return fail(format!(
                    "Interest {} has both a ## When section and buzz.interests instructions",
                    entry.id
                ));
            }
            (Some(text), None) => tidy(text).to_owned(),
            (None, Some(text)) => text,
            (None, None) => {
                return fail(format!(
                    "Interest {} has no instructions. Add a ## When section for its wakes.",
                    entry.id
                ));
            }
        };
        if text.is_empty() {
            return fail(format!("Interest {} has empty instructions", entry.id));
        }
        objects.insert(
            format!("{INTEREST}{}", entry.id),
            AttentionValue::Interest(Interest {
                title: entry.title.clone(),
                summary: entry.summary.clone(),
                instructions: text,
                paused: entry.paused,
            }),
        );
    }
    for (id, text) in instructions {
        objects.insert(
            format!("{INTEREST}{id}"),
            AttentionValue::Interest(Interest {
                title: None,
                summary: None,
                instructions: text,
                paused: None,
            }),
        );
    }
    for (wake_id, interest) in &resolved {
        if !objects.contains_key(&format!("{INTEREST}{interest}")) {
            return fail(format!(
                "Wake {wake_id} feeds Interest {interest}, which has no ## When section"
            ));
        }
    }

    for timer in &wake.timers {
        objects.insert(
            format!("{WATCH}{}", timer.id),
            AttentionValue::Timer(TimerWatch {
                interest_id: resolved[timer.id.as_str()].clone(),
                prompt: timer.prompt.clone(),
                enabled: timer.enabled,
                interval_secs: parse_every(&timer.every)
                    .map_err(|e| CodecError(format!("wake.timers[{}]: {}", timer.id, e.0)))?,
                max_occurrences: match timer.max {
                    Max::Count(0) => {
                        return fail(format!("wake.timers[{}]: max must be at least 1", timer.id));
                    }
                    Max::Count(count) => Some(count),
                    Max::Unbounded(_) => None,
                },
                expires_at: timer.expires_at,
                channel: timer.channel.clone(),
                root: timer.root.clone(),
            }),
        );
    }
    for event in &wake.events {
        objects.insert(
            format!("{WATCH}{}", event.id),
            AttentionValue::Event(EventWatch {
                interest_id: resolved[event.id.as_str()].clone(),
                name: event.name.clone(),
                enabled: event.enabled,
                channel: event.channel.clone(),
                root: event.root.clone(),
                channels: event.channels.clone(),
                kinds: event.kinds.clone(),
                authors: event.authors.clone(),
                exclude_authors: event.exclude_authors.clone(),
                tags: event.tags.clone(),
                is_reply: event.is_reply,
                classifier: event.classifier.as_ref().map(|questions| Classifier {
                    questions: questions
                        .iter()
                        .map(|(name, spec)| (name.clone(), question(spec)))
                        .collect(),
                }),
            }),
        );
    }
    Ok(objects)
}

fn question(spec: &QuestionSpec) -> Question {
    match spec {
        QuestionSpec::Text(text) => Question {
            question: text.clone(),
            if_true: None,
            if_false: None,
            guidance: None,
            threshold: None,
        },
        QuestionSpec::Full(full) => Question {
            question: full.question.clone(),
            if_true: full.if_true.clone(),
            if_false: full.if_false.clone(),
            guidance: full.guidance.clone(),
            threshold: full.threshold,
        },
    }
}

/// The fields that differ between two documents, named as in the file.
pub fn changed_fields(old: &AgentDocument, new: &AgentDocument) -> Vec<String> {
    let mut out = Vec::new();
    if old.profile.name != new.profile.name {
        out.push("display_name".to_owned());
    }
    if old.profile.about != new.profile.about {
        out.push("description".to_owned());
    }
    if old.profile.picture != new.profile.picture {
        out.push("avatar".to_owned());
    }
    let keys: BTreeSet<&String> = old.settings.keys().chain(new.settings.keys()).collect();
    for key in keys {
        if old.settings.get(key) != new.settings.get(key) {
            out.push(if TOP_LEVEL_SETTINGS.contains(&key.as_str()) {
                key.clone()
            } else {
                format!("buzz.settings.{key}")
            });
        }
    }
    if old.agent_type != new.agent_type {
        out.push("buzz.runtime".to_owned());
    }
    if old.channels != new.channels {
        out.push("buzz.channels".to_owned());
    }
    if old.instructions != new.instructions {
        out.push("persona".to_owned());
    }
    let slugs: BTreeSet<&String> = old
        .attention
        .objects
        .keys()
        .chain(new.attention.objects.keys())
        .collect();
    for slug in slugs {
        let (a, b) = (
            old.attention.objects.get(slug),
            new.attention.objects.get(slug),
        );
        if a == b {
            continue;
        }
        let id = slug
            .strip_prefix(INTEREST)
            .or_else(|| slug.strip_prefix(WATCH))
            .unwrap_or(slug);
        out.push(match a.or(b) {
            Some(AttentionValue::Interest(_)) => format!("interest {id}"),
            Some(AttentionValue::Timer(_)) => format!("wake.timers[{id}]"),
            _ => format!("wake.events[{id}]"),
        });
    }
    out
}

/// A slug for a display name, usable as `name` and as the directory.
pub fn slug_name(display: &str) -> String {
    let mut out = String::new();
    for c in display.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.truncate(64);
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        "agent".to_owned()
    } else {
        out
    }
}

/// `name` must be lowercase letters, digits and dashes, up to 64 long.
pub fn check_name(name: &str) -> Result<(), CodecError> {
    if slug_like(name, 64) {
        Ok(())
    } else {
        fail(format!(
            "name {name:?} must be 1 to 64 lowercase letters, digits and dashes, starting with a letter or digit"
        ))
    }
}

fn check_wake_id(id: &str) -> Result<(), CodecError> {
    if slug_like(id, 40) {
        Ok(())
    } else {
        fail(format!(
            "wake id {id:?} must be 1 to 40 lowercase letters, digits and dashes, starting with a letter or digit"
        ))
    }
}

fn slug_like(s: &str, max: usize) -> bool {
    let mut chars = s.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    first_ok
        && s.len() <= max
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The largest unit that divides `secs` exactly, so the conversion is exact.
fn every(secs: u64) -> Result<String, CodecError> {
    if secs == 0 {
        return fail("A timer's interval is zero");
    }
    let (unit, size) = UNITS
        .iter()
        .find(|(_, size)| secs.is_multiple_of(*size))
        .copied()
        .unwrap_or(('s', 1));
    Ok(format!("{}{unit}", secs / size))
}

fn parse_every(text: &str) -> Result<u64, CodecError> {
    let bad = || {
        CodecError(format!(
            "every {text:?} must be a whole number and s, m, h or d"
        ))
    };
    let unit = text.chars().last().ok_or_else(bad)?;
    let size = UNITS
        .iter()
        .find(|(u, _)| *u == unit)
        .map(|(_, size)| *size)
        .ok_or_else(bad)?;
    let digits = &text[..text.len() - 1];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let count: u64 = digits.parse().map_err(|_| bad())?;
    if count == 0 {
        return Err(bad());
    }
    count.checked_mul(size).ok_or_else(bad)
}

/// Hex public key to npub.
pub fn npub(hex: &str) -> Result<String, CodecError> {
    let key = PublicKey::from_hex(hex)
        .map_err(|_| CodecError(format!("The app sent an invalid agent public key: {hex}")))?;
    key.to_bech32()
        .map_err(|e| CodecError(format!("Could not encode an npub: {e}")))
}

/// Whether the file would read `text` differently once a section follows it:
/// it has a `## When` heading outside fenced code, or leaves a fence open.
fn has_heading(text: &str) -> bool {
    let (lines, open) = fenced_lines(text);
    open || lines.iter().any(|(line, fenced)| {
        !fenced && section_heading(line.trim_end_matches(['\n', '\r'])).is_some()
    })
}

/// Drops leading blank lines and trailing whitespace, keeping indentation.
fn tidy(text: &str) -> &str {
    let text = text.trim_end();
    let start = text
        .char_indices()
        .find(|(_, c)| !c.is_whitespace())
        .map_or(text.len(), |(i, _)| i);
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    &text[line_start..]
}

fn take_text(
    settings: &mut serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, CodecError> {
    match settings.remove(key) {
        // An empty setting is the type's default, as a left-out one is.
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text).filter(|text| !text.is_empty())),
        Some(other) => fail(format!("The agent's {key} is not text: {other}")),
    }
}

fn json_to_yaml(value: &Value) -> Result<serde_yaml::Value, CodecError> {
    serde_yaml::to_value(value).map_err(|e| CodecError(format!("Could not write a setting: {e}")))
}

fn yaml_to_json(value: &serde_yaml::Value) -> Result<Value, CodecError> {
    serde_json::to_value(value).map_err(|e| CodecError(format!("buzz.settings: {e}")))
}
