use nostr::Tag;

const MAX_TEAM_MENTION_FIELD_CHARS: usize = 200;

pub(super) fn parse_mention_reference_tag(mention: &[String]) -> Result<Tag, String> {
    match mention.first().map(String::as_str) {
        Some("mention") => parse_person_mention(mention),
        Some("team_mention") => parse_team_mention(mention),
        prefix => Err(format!(
            "mention reference tags must use 'mention' or 'team_mention' prefix (got {prefix:?})"
        )),
    }
}

fn parse_person_mention(mention: &[String]) -> Result<Tag, String> {
    if mention.len() != 2 {
        return Err("mention reference tag must contain exactly one pubkey".into());
    }
    let pubkey = &mention[1];
    if pubkey.len() != 64
        || !pubkey
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!(
            "pubkey must be a 64-character hex string (got {} chars)",
            pubkey.len()
        ));
    }
    Tag::parse(["mention", &pubkey.to_ascii_lowercase()])
        .map_err(|error| format!("invalid tag: {error}"))
}

fn parse_team_mention(mention: &[String]) -> Result<Tag, String> {
    if mention.len() != 3 {
        return Err("team mention tag must contain exactly a team id and display name".into());
    }
    let team_id = mention[1].trim();
    let display_name = mention[2].trim();
    if team_id.is_empty() || display_name.is_empty() {
        return Err("team mention id and display name must not be blank".into());
    }
    if team_id.chars().count() > MAX_TEAM_MENTION_FIELD_CHARS
        || display_name.chars().count() > MAX_TEAM_MENTION_FIELD_CHARS
    {
        return Err("team mention id and display name must be at most 200 characters".into());
    }
    if team_id.chars().any(char::is_control) || display_name.chars().any(char::is_control) {
        return Err("team mention id and display name must not contain controls".into());
    }
    Tag::parse(["team_mention", team_id, display_name])
        .map_err(|error| format!("invalid tag: {error}"))
}

#[cfg(test)]
mod tests {
    use super::parse_mention_reference_tag;

    #[test]
    fn preserves_visible_team_chip_metadata() {
        let tag = parse_mention_reference_tag(&[
            "team_mention".into(),
            "team-launch".into(),
            "Launch Team".into(),
        ])
        .unwrap();
        assert_eq!(
            tag.as_slice(),
            &["team_mention", "team-launch", "Launch Team"]
        );
    }

    #[test]
    fn rejects_extra_team_tag_fields() {
        assert!(parse_mention_reference_tag(&[
            "team_mention".into(),
            "team-launch".into(),
            "Launch Team".into(),
            "forged".into(),
        ])
        .is_err());
    }
}
