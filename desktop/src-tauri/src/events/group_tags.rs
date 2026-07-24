use nostr::Tag;
use uuid::Uuid;

pub(super) fn append_group_marker_tags(
    groups: &[Vec<String>],
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    for group in groups {
        if group.first().map(String::as_str) != Some("group") {
            return Err(format!(
                "group marker tags must use 'group' prefix (got {:?})",
                group.first()
            ));
        }
        if group.len() != 3 || group[1].is_empty() || group[2].is_empty() {
            return Err("group marker tag must be [\"group\", id, handle]".into());
        }
        Uuid::parse_str(&group[1]).map_err(|_| "group marker tag has invalid UUID")?;
        let handle = group[2].as_bytes();
        if !(2..=32).contains(&handle.len())
            || !handle[0].is_ascii_lowercase() && !handle[0].is_ascii_digit()
            || handle.iter().any(|byte| {
                !byte.is_ascii_lowercase()
                    && !byte.is_ascii_digit()
                    && *byte != b'_'
                    && *byte != b'-'
            })
        {
            return Err("group marker tag has invalid handle".into());
        }
        tags.push(
            Tag::parse(["group", group[1].as_str(), group[2].as_str()])
                .map_err(|error| format!("invalid group marker tag: {error}"))?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::append_group_marker_tags;

    #[test]
    fn validates_and_preserves_marker_shape() {
        let mut tags = Vec::new();
        append_group_marker_tags(
            &[vec![
                "group".into(),
                "11111111-1111-4111-8111-111111111111".into(),
                "ios-team".into(),
            ]],
            &mut tags,
        )
        .unwrap();
        let marker = tags[0].as_slice();
        assert_eq!(marker[0], "group");
        assert_eq!(marker[1], "11111111-1111-4111-8111-111111111111");
        assert_eq!(marker[2], "ios-team");
    }

    #[test]
    fn rejects_wrong_prefix_and_malformed_shape() {
        assert!(append_group_marker_tags(
            &[vec![
                "p".into(),
                "11111111-1111-4111-8111-111111111111".into(),
                "ios-team".into(),
            ]],
            &mut Vec::new(),
        )
        .is_err());
        assert!(append_group_marker_tags(
            &[vec!["group".into(), "not-a-uuid".into(), "ios-team".into()]],
            &mut Vec::new(),
        )
        .is_err());
        assert!(append_group_marker_tags(
            &[vec![
                "group".into(),
                "11111111-1111-4111-8111-111111111111".into(),
                "Invalid Handle".into(),
            ]],
            &mut Vec::new(),
        )
        .is_err());
    }
}
