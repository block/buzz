//! Which `@name` text in a message the Buzz apps show as a tag.
//!
//! `buzz messages send` resolves `@name` against the same five names the
//! desktop's agent `send` tool does (buzz-app `src/buzz-mcp/tools.ts`,
//! `aliases`), and refuses a message in which an `@name` that reads as someone
//! it tags would show as plain text. The apps bind a tag only by a person's
//! shown name, verified display name and verified full name, written exactly
//! (buzz-app `src/features/messages/profile-mentions.ts`), so a name someone
//! gave their own profile, a verified username or a case change can notify a
//! person yet not read as their tag.
//!
//! The two implementations cannot share code. They stay in step through the
//! same test cases and the same error text: keep the tests below in line with
//! buzz-app's "refuses an @name that would notify someone it tags but not show
//! as their tag" test.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::verified_profiles::VerifiedProfile;

/// Lower wins when people share a name. A relay-verified name outranks a name
/// someone gave their own profile, so a profile cannot take a verified
/// person's name. Same order as buzz-app's `NAME_WEIGHT`.
const VERIFIED_USERNAME: u8 = 0;
const VERIFIED_DISPLAY_NAME: u8 = 1;
const VERIFIED_FULL_NAME: u8 = 2;
const DISPLAY_NAME: u8 = 3;
const NAME: u8 = 4;

/// Bodies longer than this (UTF-16 units) render as plain text, with no tags,
/// so there is nothing to write instead. buzz-app `MAX_MARKDOWN_LENGTH`.
const MAX_MARKDOWN_LENGTH: usize = 100_000;

/// A channel member's names: their own latest kind:0 and their verified
/// profile, either of which may be missing.
#[derive(Debug, Clone, Default)]
pub(crate) struct Person {
    /// Lowercase hex.
    pub pubkey: String,
    /// The kind:0 content parsed as a JSON object.
    pub profile_parsed: bool,
    pub display_name: Option<String>,
    pub name: Option<String>,
    pub verified: Option<VerifiedProfile>,
}

/// Each member's newest kind:0 from `events`, with their verified profile,
/// then any member known only by a verified profile.
pub(crate) fn people_from(
    events: &[Value],
    verified: &HashMap<String, VerifiedProfile>,
) -> Vec<Person> {
    let mut newest: Vec<(String, u64, &Value)> = Vec::new();
    for event in events {
        let Some(pubkey) = event.get("pubkey").and_then(Value::as_str) else {
            continue;
        };
        let pubkey = pubkey.to_ascii_lowercase();
        let created = event.get("created_at").and_then(Value::as_u64).unwrap_or(0);
        match newest.iter_mut().find(|(key, _, _)| *key == pubkey) {
            Some(entry) if created > entry.1 => *entry = (pubkey, created, event),
            Some(_) => {}
            None => newest.push((pubkey, created, event)),
        }
    }
    let mut people: Vec<Person> = newest
        .into_iter()
        .map(|(pubkey, _, event)| {
            let body = event
                .get("content")
                .and_then(Value::as_str)
                .and_then(|content| serde_json::from_str::<Value>(content).ok())
                .filter(Value::is_object);
            let text = |field: &str| {
                body.as_ref()
                    .and_then(|body| body.get(field))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            Person {
                verified: verified.get(&pubkey).cloned(),
                profile_parsed: body.is_some(),
                display_name: text("display_name"),
                name: text("name"),
                pubkey,
            }
        })
        .collect();
    let mut extra: Vec<(&String, &VerifiedProfile)> = verified
        .iter()
        .filter(|(key, _)| !people.iter().any(|person| &person.pubkey == *key))
        .collect();
    extra.sort_by(|a, b| a.0.cmp(b.0));
    people.extend(extra.into_iter().map(|(pubkey, found)| Person {
        pubkey: pubkey.clone(),
        verified: Some(found.clone()),
        ..Person::default()
    }));
    people
}

/// Every name `send` accepts after `@`, lowercase, with each holder's best
/// weight, holders in the order they were found.
pub(crate) type Aliases = HashMap<String, Vec<(String, u8)>>;

pub(crate) fn aliases(people: &[Person]) -> Aliases {
    let mut aliases = Aliases::new();
    let mut add = |pubkey: &str, name: Option<&str>, weight: u8| {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            return;
        };
        let holders = aliases.entry(name.to_lowercase()).or_default();
        match holders.iter_mut().find(|(key, _)| key == pubkey) {
            Some(holder) => holder.1 = holder.1.min(weight),
            None => holders.push((pubkey.to_owned(), weight)),
        }
    };
    for person in people {
        add(&person.pubkey, person.display_name.as_deref(), DISPLAY_NAME);
        add(&person.pubkey, person.name.as_deref(), NAME);
    }
    for person in people {
        if let Some(found) = &person.verified {
            add(
                &person.pubkey,
                Some(&found.display_name),
                VERIFIED_DISPLAY_NAME,
            );
            add(&person.pubkey, Some(&found.username), VERIFIED_USERNAME);
            add(
                &person.pubkey,
                found.full_name.as_deref(),
                VERIFIED_FULL_NAME,
            );
        }
    }
    aliases
}

/// The holders of a name with its best weight.
pub(crate) fn best_named(holders: Option<&Vec<(String, u8)>>) -> Vec<String> {
    let Some(holders) = holders.filter(|holders| !holders.is_empty()) else {
        return vec![];
    };
    let best = holders.iter().map(|(_, weight)| *weight).min().unwrap_or(0);
    holders
        .iter()
        .filter(|(_, weight)| *weight == best)
        .map(|(key, _)| key.clone())
        .collect()
}

/// An `@` at a word start: its byte offset, the longest name in the aliases
/// it starts with (lowercase), and that name or else the word that follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AtName {
    pub at: usize,
    pub name: Option<String>,
    pub token: Option<String>,
}

/// Each `@` in `prose` at a word start, as the agent `send` tool reads it.
/// Run it on [`without_code`] output: an `@` in code tags no one.
pub(crate) fn at_names(prose: &str, aliases: &Aliases) -> Vec<AtName> {
    let mut longest: Vec<&String> = aliases.keys().collect();
    longest.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    let mut found = Vec::new();
    for (at, _) in prose.match_indices('@') {
        if prose[..at]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
        {
            continue;
        }
        let rest = &prose[at + 1..];
        let name = longest.iter().find(|candidate| {
            rest.get(..candidate.len())
                .is_some_and(|start| start.to_lowercase() == ***candidate)
                && ends_name(&rest[candidate.len()..])
        });
        let token = match name {
            Some(name) => Some((*name).clone()),
            None => {
                let lower = rest.to_lowercase();
                let end = lower
                    .find(|c: char| !matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-'))
                    .unwrap_or(lower.len());
                (end > 0).then(|| lower[..end].to_owned())
            }
        };
        found.push(AtName {
            at,
            name: name.map(|name| (*name).clone()),
            token,
        });
    }
    found
}

fn ends_name(rest: &str) -> bool {
    rest.chars().next().is_none_or(|c| {
        c.is_whitespace() || matches!(c, ',' | ';' | '.' | '!' | '?' | ':' | ')' | ']' | '}')
    })
}

/// `content` with fenced and inline code blanked to spaces, byte offsets
/// kept. Same spans as the agent `send` tool blanks.
pub(crate) fn without_code(content: &str) -> String {
    let mut bytes = content.as_bytes().to_vec();
    // Fenced: "```" at a line start through the next "```" at a line start.
    let mut search = 0;
    while let Some(open) = find_at_line_start(content, "```", search) {
        let Some(close) = find_at_line_start(content, "```", open + 3) else {
            break;
        };
        let end = close + 3;
        for byte in &mut bytes[open..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        search = end;
    }
    // Inline: a backtick, then no backtick or newline, then a backtick.
    let mut at = 0;
    while let Some(offset) = bytes[at..].iter().position(|b| *b == b'`') {
        let open = at + offset;
        let close = bytes[open + 1..]
            .iter()
            .position(|b| *b == b'`' || *b == b'\n')
            .map(|offset| open + 1 + offset);
        match close {
            Some(close) if bytes[close] == b'`' => {
                for byte in &mut bytes[open..=close] {
                    *byte = b' ';
                }
                at = close + 1;
            }
            _ => at = open + 1,
        }
    }
    // Every replaced byte became an ASCII space and whole characters are
    // replaced, so the result is still UTF-8.
    String::from_utf8(bytes).unwrap_or_else(|_| content.to_owned())
}

fn find_at_line_start(text: &str, needle: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while let Some(offset) = text.get(search..)?.find(needle) {
        let at = search + offset;
        if at == 0 || text.as_bytes()[at - 1] == b'\n' {
            return Some(at);
        }
        search = at + 1;
    }
    None
}

/// A person as the apps show them to every reader: a verified person under
/// their verified display name, anyone else under the name their profile
/// gives, or the start of their key.
struct Shown<'a> {
    name: String,
    fallback: bool,
    verified: Option<&'a VerifiedProfile>,
}

fn shown(person: &Person) -> Shown<'_> {
    if let Some(found) = &person.verified {
        return Shown {
            name: found.display_name.clone(),
            fallback: false,
            verified: Some(found),
        };
    }
    let name = person
        .profile_parsed
        .then(|| {
            profile_name(person.display_name.as_deref())
                .or_else(|| profile_name(person.name.as_deref()))
        })
        .flatten();
    match name {
        Some(name) => Shown {
            name,
            fallback: false,
            verified: None,
        },
        None => Shown {
            name: person.pubkey.chars().take(10).collect(),
            fallback: true,
            verified: None,
        },
    }
}

/// buzz-app `profileName`: trimmed, and no control, separator or
/// reordering characters.
fn profile_name(value: Option<&str>) -> Option<String> {
    let name = value?.trim();
    let bad = |c: char| {
        c.is_control()
            || matches!(
                c,
                '\u{2028}' | '\u{2029}' | '\u{061c}' | '\u{200e}' | '\u{200f}'
            )
            || ('\u{202a}'..='\u{202e}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c)
            || ('\u{fff9}'..='\u{fffb}').contains(&c)
    };
    (!name.is_empty() && !name.chars().any(bad)).then(|| name.to_owned())
}

/// The names a tag binds by: buzz-app `profileMentionNames`.
fn mention_names(shown: &Shown<'_>) -> Vec<String> {
    let mut names = vec![shown.name.clone()];
    if let Some(found) = shown.verified {
        names.push(found.display_name.clone());
        names.extend(found.full_name.clone());
    }
    names
}

/// The pubkey each `@` in `content` shows as a tag for, by byte offset: a
/// port of buzz-app's `profileMentionParts` for an unedited body with no
/// attachments. Unlike the app it doesn't parse Markdown, so it uses only the
/// renderer's own exclusions (code, links, URLs).
fn bound_tags(content: &str, keys: &[String], people: &[Person]) -> HashMap<usize, String> {
    let mut names: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen = HashSet::new();
    for key in keys {
        if !seen.insert(key) {
            continue;
        }
        let Some(person) = people.iter().find(|person| &person.pubkey == key) else {
            continue;
        };
        let mut labels = mention_names(&shown(person));
        labels.dedup();
        for name in labels {
            if name.is_empty() || name.contains(['\r', '\n']) {
                continue;
            }
            match names.iter_mut().find(|(known, _)| *known == name) {
                Some((_, holders)) if holders.contains(key) => {}
                Some((_, holders)) => holders.push(key.clone()),
                None => names.push((name, vec![key.clone()])),
            }
        }
    }
    // Ambiguous long names still consume their span, never a prefix.
    names.sort_by_key(|(name, _)| std::cmp::Reverse(utf16_len(name)));

    let mut bound = HashMap::new();
    let mut i = 0;
    while i < content.len() {
        let protected = protected_text_end(content, i);
        if protected > i {
            i = protected;
            continue;
        }
        let here = content[i..].chars().next().unwrap_or(' ');
        let before = content[..i].chars().next_back();
        if here != '@'
            || before.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '@' | '/' | '\\'))
        {
            i += here.len_utf8();
            continue;
        }
        let after = &content[i + 1..];
        // A qualified span names its signed key, so its name may differ in case.
        let qualified = names.iter().find_map(|(name, holders)| {
            let start = after.get(..name.len())?;
            if start.to_lowercase() != name.to_lowercase() {
                return None;
            }
            let (target, length) = qualifier(&after[name.len()..])?;
            holders
                .contains(&target)
                .then(|| (i + 1 + name.len() + length, target))
        });
        if let Some((end, target)) = qualified {
            bound.insert(i, target);
            i = end;
            continue;
        }
        let candidate = names.iter().find(|(name, _)| {
            after.starts_with(name.as_str())
                && !after[name.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
        });
        let Some((name, holders)) = candidate else {
            i += 1;
            continue;
        };
        let end = i + 1 + name.len();
        // A qualifier no signed key with this name carries stays literal.
        if let Some((_, length)) = qualifier(&content[end..]) {
            i = end + length;
            continue;
        }
        if let [target] = holders.as_slice() {
            bound.insert(i, target.clone());
        }
        i = end;
    }
    bound
}

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// ` (<64 hex>)` at the start of `rest`: the lowercase key and the span's
/// byte length.
fn qualifier(rest: &str) -> Option<(String, usize)> {
    let hex = rest.strip_prefix(" (")?.get(..64)?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) || rest.as_bytes().get(66) != Some(&b')') {
        return None;
    }
    Some((hex.to_ascii_lowercase(), 67))
}

/// Where a run the renderer never reads mentions in ends, or `start`: indented
/// and fenced code, inline code, links and URLs. buzz-app `protectedTextEnd`.
fn protected_text_end(text: &str, start: usize) -> usize {
    let rest = &text[start..];
    let line_start = start == 0 || text.as_bytes()[start - 1] == b'\n';
    if line_start {
        let line_end = rest.find('\n').map(|offset| start + offset);
        let spaces = rest.bytes().take_while(|b| *b == b' ').count();
        if spaces >= 4 || (spaces <= 3 && rest.as_bytes().get(spaces) == Some(&b'\t')) {
            return line_end.map_or(text.len(), |end| end + 1);
        }
        if spaces <= 3 {
            let marks = &rest[spaces..];
            let fence_char = marks.chars().next().filter(|c| matches!(c, '`' | '~'));
            if let Some(fence_char) = fence_char {
                let length = marks.chars().take_while(|c| *c == fence_char).count();
                if length >= 3 {
                    let Some(line_end) = line_end else {
                        return text.len();
                    };
                    return closing_fence_end(text, line_end + 1, fence_char, length)
                        .unwrap_or(text.len());
                }
            }
        }
    }
    let first = rest.chars().next().unwrap_or(' ');
    let boundary = start == 0
        || text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_whitespace() || matches!(c, '<' | '(' | '['));
    if !matches!(first, '`' | '~' | '[' | '!') && !boundary {
        return start;
    }
    let backticks = rest.bytes().take_while(|b| *b == b'`').count();
    let tildes = rest.bytes().take_while(|b| *b == b'~').count();
    let span = if backticks > 0 {
        Some(&rest[..backticks])
    } else if tildes >= 3 {
        Some(&rest[..tildes])
    } else {
        None
    };
    if let Some(span) = span {
        return text[start + span.len()..]
            .find(span)
            .map_or(text.len(), |offset| {
                start + span.len() + offset + span.len()
            });
    }
    if rest.starts_with('[') || rest.starts_with("![") {
        if let Some(length) = link_length(rest) {
            return start + length;
        }
    }
    url_length(rest).map_or(start, |length| start + length)
}

/// The end of the first line at or after `from` that closes a fence: up to
/// three spaces, at least `length` of `fence_char`, then only spaces or tabs.
fn closing_fence_end(text: &str, from: usize, fence_char: char, length: usize) -> Option<usize> {
    let mut line_start = from;
    while line_start <= text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |offset| line_start + offset);
        let line = &text[line_start..line_end];
        let spaces = line.bytes().take_while(|b| *b == b' ').count();
        if spaces <= 3 {
            let marks = line[spaces..]
                .chars()
                .take_while(|c| *c == fence_char)
                .count();
            let tail = &line[spaces + marks * fence_char.len_utf8()..];
            let tail = tail.strip_suffix('\r').unwrap_or(tail);
            if marks >= length && tail.bytes().all(|b| b == b' ' || b == b'\t') {
                return Some(line_end);
            }
        }
        if line_end == text.len() {
            return None;
        }
        line_start = line_end + 1;
    }
    None
}

/// `^!?\[[^\]\n]*\](?:\([^\n]*?\)|\[[^\]\n]*\])`
fn link_length(rest: &str) -> Option<usize> {
    let body = rest.strip_prefix('!').unwrap_or(rest);
    let bang = rest.len() - body.len();
    let label_end = 1 + body[1..].find([']', '\n'])?;
    if body.as_bytes()[label_end] != b']' {
        return None;
    }
    let after = &body[label_end + 1..];
    let (open, close) = match after.as_bytes().first()? {
        b'(' => (b'(', ')'),
        b'[' => (b'[', ']'),
        _ => return None,
    };
    let inner = &after[1..];
    let end = if open == b'(' {
        let line = inner.find('\n').map_or(inner, |end| &inner[..end]);
        line.find(close)?
    } else {
        let stop = inner.find([']', '\n'])?;
        (inner.as_bytes()[stop] == b']').then_some(stop)?
    };
    Some(bang + label_end + 1 + 1 + end + 1)
}

/// `^(?:[a-z][a-z\d+.-]*://|mailto:|nostr:)[^\s<>`]+`, case-insensitive.
fn url_length(rest: &str) -> Option<usize> {
    let lower = rest.to_ascii_lowercase();
    let prefix = if lower.starts_with("mailto:") {
        7
    } else if lower.starts_with("nostr:") {
        6
    } else {
        let mut chars = lower.char_indices();
        let (_, first) = chars.next()?;
        if !first.is_ascii_lowercase() {
            return None;
        }
        let scheme_end = chars
            .find(|(_, c)| {
                !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '.' | '-'))
            })
            .map_or(lower.len(), |(at, _)| at);
        if !lower[scheme_end..].starts_with("://") {
            return None;
        }
        scheme_end + 3
    };
    let body = rest[prefix..]
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '`'))
        .unwrap_or(rest.len() - prefix);
    (body > 0).then_some(prefix + body)
}

/// Refuses `content` when an `@name` that `send` reads as someone in `keys`
/// would show as plain text. The error names what to write instead, in the
/// same words as the agent `send` tool.
pub(crate) fn untagged_names(
    content: &str,
    keys: &[String],
    aliases: &Aliases,
    people: &[Person],
) -> Result<(), String> {
    if utf16_len(content) > MAX_MARKDOWN_LENGTH {
        return Ok(());
    }
    let bound = bound_tags(content, keys, people);
    let mut dead: Vec<(String, Vec<String>)> = Vec::new();
    for AtName { at, name, .. } in at_names(&without_code(content), aliases) {
        let tagged: Vec<String> = name
            .as_ref()
            .and_then(|name| aliases.get(name))
            .map(|holders| {
                holders
                    .iter()
                    .map(|(key, _)| key.clone())
                    .filter(|key| keys.contains(key))
                    .collect()
            })
            .unwrap_or_default();
        if tagged.is_empty() || bound.get(&at).is_some_and(|target| tagged.contains(target)) {
            continue;
        }
        let length = name.as_ref().map_or(0, String::len);
        let written = content.get(at..at + 1 + length).unwrap_or("@").to_owned();
        match dead.iter_mut().find(|(known, _)| *known == written) {
            Some((_, people)) => {
                for key in tagged {
                    if !people.contains(&key) {
                        people.push(key);
                    }
                }
            }
            None => dead.push((written, tagged)),
        }
    }
    if dead.is_empty() {
        return Ok(());
    }
    let tags = shown_tags(keys, people);
    let fixes: Vec<String> = dead
        .iter()
        .map(|(written, tagged)| {
            let fixes: Vec<String> = tagged
                .iter()
                .map(|key| {
                    let person = people.iter().find(|person| &person.pubkey == key);
                    let who = person_name(person, key);
                    match tags.get(key) {
                        Some(tag) => format!("{who} is {tag}"),
                        None => format!(
                            "{who} has no name that shows as a tag, so drop the @ and keep their pubkey in mentions"
                        ),
                    }
                })
                .collect();
            format!("{written}: {}", fixes.join("; "))
        })
        .collect();
    Err(format!(
        "These would notify but not show as tags: {}. Nothing was posted.",
        fixes.join(". ")
    ))
}

/// What to write to tag each of `keys`: `@` and the name the apps show them
/// by, qualified with their key when another of `keys` binds the same name.
fn shown_tags(keys: &[String], people: &[Person]) -> HashMap<String, String> {
    let find = |key: &String| people.iter().find(|person| &person.pubkey == key);
    let mut tags = HashMap::new();
    for key in keys {
        let Some(person) = find(key) else {
            continue;
        };
        let shown = shown(person);
        if shown.fallback || shown.name.is_empty() {
            continue;
        }
        let name = shown.name.to_lowercase();
        let shared = keys.iter().any(|other| {
            other != key
                && find(other).is_some_and(|other| {
                    mention_names(&self::shown(other))
                        .iter()
                        .any(|same| same.to_lowercase() == name)
                })
        });
        let tag = if shared {
            format!("@{} ({key})", shown.name)
        } else {
            format!("@{}", shown.name)
        };
        tags.insert(key.clone(), tag);
    }
    tags
}

fn person_name(person: Option<&Person>, key: &str) -> String {
    let Some(person) = person else {
        return key.to_owned();
    };
    let shown = shown(person);
    if let Some(found) = shown.verified {
        return found
            .full_name
            .clone()
            .unwrap_or_else(|| found.display_name.clone());
    }
    if shown.fallback {
        key.to_owned()
    } else {
        shown.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SAM: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    const BOB: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CAROL: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn profile(pubkey: &str, created_at: u64, body: Value) -> Value {
        json!({ "pubkey": pubkey, "created_at": created_at, "kind": 0, "content": body.to_string() })
    }

    fn verified(display_name: &str, username: &str, full_name: Option<&str>) -> VerifiedProfile {
        VerifiedProfile {
            display_name: display_name.into(),
            username: username.into(),
            full_name: full_name.map(Into::into),
            issuer_label: "Example".into(),
            verified_at: 100,
        }
    }

    /// buzz-app's test world: Sam's own profile says "Sam"; the relay verified
    /// the handle `srivera` and the full name "Sam Rivera".
    fn world(extra: &[Value]) -> Vec<Person> {
        let mut events = vec![
            profile(BOB, 100, json!({ "display_name": "Bob" })),
            profile(CAROL, 100, json!({ "display_name": "Carol" })),
            profile(SAM, 100, json!({ "display_name": "Sam" })),
        ];
        events.extend_from_slice(extra);
        let verified = HashMap::from([(
            SAM.to_owned(),
            verified("srivera", "srivera", Some("Sam Rivera")),
        )]);
        people_from(&events, &verified)
    }

    /// What `send` would do: resolve `@name` as the CLI does, then check tags.
    fn check(people: &[Person], text: &str, explicit: &[&str]) -> Result<Vec<String>, String> {
        let aliases = aliases(people);
        let mut keys: Vec<String> = explicit.iter().map(|key| (*key).to_owned()).collect();
        for at in at_names(&without_code(text), &aliases) {
            if let [key] =
                best_named(at.name.as_ref().and_then(|name| aliases.get(name))).as_slice()
            {
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
        untagged_names(text, &keys, &aliases, people).map(|()| keys)
    }

    #[test]
    fn own_profile_name_notifies_but_is_refused_with_the_tag_to_write() {
        let people = world(&[]);
        for explicit in [&[SAM][..], &[][..]] {
            assert_eq!(
                check(&people, "@Sam can you re-approve?", explicit).unwrap_err(),
                "These would notify but not show as tags: @Sam: Sam Rivera is @srivera. Nothing was posted."
            );
        }
    }

    #[test]
    fn a_case_change_of_a_verified_name_is_refused() {
        let error = check(&world(&[]), "@sam rivera hi", &[SAM]).unwrap_err();
        assert!(
            error.contains("@sam rivera: Sam Rivera is @srivera."),
            "{error}"
        );
    }

    #[test]
    fn verified_names_written_exactly_are_accepted() {
        for text in ["@srivera can you re-approve?", "@Sam Rivera hi"] {
            for explicit in [&[SAM][..], &[][..]] {
                assert_eq!(
                    check(&world(&[]), text, explicit).unwrap(),
                    vec![SAM.to_owned()]
                );
            }
        }
    }

    #[test]
    fn text_naming_no_one_tagged_and_code_are_left_alone() {
        check(&world(&[]), "@Carol and @srivera, see `@Sam`", &[SAM]).unwrap();
        check(&world(&[]), "ping @nobody\n```\n@Sam\n```\n", &[SAM]).unwrap();
        // Carol is a member but not tagged: her name is plain text, as before.
        assert_eq!(
            check(&world(&[]), "@srivera, ask Carol", &[]).unwrap(),
            vec![SAM.to_owned()]
        );
    }

    #[test]
    fn an_imitated_name_gets_the_form_that_names_one_key() {
        let people = world(&[profile(BOB, 101, json!({ "display_name": "Sam Rivera" }))]);
        assert_eq!(
            check(&people, "@Sam Rivera hi", &[SAM, BOB]).unwrap_err(),
            format!(
                "These would notify but not show as tags: @Sam Rivera: Sam Rivera is @Sam Rivera ({BOB}); Sam Rivera is @srivera. Nothing was posted."
            )
        );
        check(
            &people,
            &format!("@srivera and @Sam Rivera ({BOB}) hi"),
            &[SAM, BOB],
        )
        .unwrap();
    }

    #[test]
    fn a_username_unlike_the_display_name_suggests_the_display_name() {
        // buzz-app's Alice case: verified display name "Alice S", username
        // `asmith`, full name "Alice Smith-Jones".
        let alice = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let events = vec![profile(
            alice,
            100,
            json!({ "display_name": "Alice Smith" }),
        )];
        let verified = HashMap::from([(
            alice.to_owned(),
            verified("Alice S", "asmith", Some("Alice Smith-Jones")),
        )]);
        let people = people_from(&events, &verified);
        for text in ["@Alice S see this", "@Alice Smith-Jones see this"] {
            check(&people, text, &[alice]).unwrap();
        }
        for text in ["@asmith see this", "@alice smith-jones see this"] {
            let error = check(&people, text, &[alice]).unwrap_err();
            assert!(error.contains("Alice Smith-Jones is @Alice S."), "{error}");
        }
    }

    #[test]
    fn someone_known_only_by_key_has_no_tag_to_suggest() {
        // A display name with a reordering control is never shown, so the
        // apps show only the start of the key, and there is no name to suggest.
        let nameless = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        let people = people_from(
            &[profile(
                nameless,
                100,
                json!({ "display_name": "Ann\u{200e}" }),
            )],
            &HashMap::new(),
        );
        assert_eq!(
            check(&people, "@Ann\u{200e} hi", &[nameless]).unwrap_err(),
            format!(
                "These would notify but not show as tags: @Ann\u{200e}: {nameless} has no name that shows as a tag, so drop the @ and keep their pubkey in mentions. Nothing was posted."
            )
        );
    }

    #[test]
    fn mentions_in_links_and_urls_do_not_bind() {
        let people = world(&[]);
        let bound = bound_tags("[@srivera](https://x) @srivera", &[SAM.to_owned()], &people);
        assert_eq!(bound.len(), 1);
        assert_eq!(bound.get(&22), Some(&SAM.to_owned()));
    }

    #[test]
    fn without_code_keeps_offsets() {
        let text = "a `@Sam` é\n```\n@Sam\n```\n@x";
        let blanked = without_code(text);
        assert_eq!(blanked.len(), text.len());
        assert!(!blanked.contains("@Sam"));
        assert!(blanked.ends_with("@x"));
    }
}
