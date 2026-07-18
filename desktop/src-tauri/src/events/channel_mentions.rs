use nostr::Tag;

use super::{check_pubkey, tag};

pub(super) fn append(metadata: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for item in metadata {
        match item.first().map(String::as_str) {
            Some("mention") => {
                let Some(pubkey) = item.get(1) else {
                    return Err("mention reference tag missing pubkey".into());
                };
                if item.len() != 2 {
                    return Err("mention reference tag must have exactly two fields".into());
                }
                check_pubkey(pubkey)?;
                tags.push(tag(vec!["mention", &pubkey.to_ascii_lowercase()])?);
            }
            Some("buzz-audience-ref") => {
                let mode = validate_mode(item.get(1))?;
                if item.len() != 2 {
                    return Err("buzz-audience-ref tag must have exactly two fields".into());
                }
                tags.push(tag(vec!["buzz-audience-ref", mode])?);
            }
            Some("p")
                if item
                    .get(3)
                    .is_some_and(|marker| marker.starts_with("buzz:audience:")) =>
            {
                let Some(pubkey) = item.get(1) else {
                    return Err("channel mention p tag missing recipient pubkey".into());
                };
                if item.len() != 4 || !item[2].is_empty() {
                    return Err("channel mention p tag must have exactly four fields".into());
                }
                check_pubkey(pubkey)?;
                let mode = validate_recipient_marker(item.get(3))?;
                let expected_marker = format!("buzz:audience:{mode}");
                if item[3] != expected_marker {
                    return Err("channel mention p tag marker does not match its mode".into());
                }
                tags.push(tag(vec![
                    "p",
                    &pubkey.to_ascii_lowercase(),
                    "",
                    &expected_marker,
                ])?);
            }
            prefix => {
                return Err(format!(
                    "mention metadata tag has unsupported prefix {prefix:?}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_recipient_marker(marker: Option<&String>) -> Result<&str, String> {
    match marker.map(String::as_str) {
        Some("buzz:audience:everyone") => Ok("everyone"),
        Some("buzz:audience:here") => Ok("here"),
        _ => Err("channel mention p tag has an unsupported marker".into()),
    }
}

fn validate_mode(mode: Option<&String>) -> Result<&str, String> {
    match mode.map(String::as_str) {
        Some("everyone") => Ok("everyone"),
        Some("here") => Ok("here"),
        _ => Err("channel mention mode must be 'everyone' or 'here'".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_standard_p_tag_with_audience_marker() {
        let pubkey = "a".repeat(64);
        let metadata = vec![
            vec!["buzz-audience-ref".into(), "here".into()],
            vec![
                "p".into(),
                pubkey.clone(),
                String::new(),
                "buzz:audience:here".into(),
            ],
        ];
        let mut tags = Vec::new();

        append(&metadata, &mut tags).expect("valid audience tags");

        assert_eq!(tags[0].as_slice(), ["buzz-audience-ref", "here"]);
        assert_eq!(
            tags[1].as_slice(),
            ["p", pubkey.as_str(), "", "buzz:audience:here"]
        );
    }

    #[test]
    fn rejects_unmarked_p_tag_in_metadata_channel() {
        let metadata = vec![vec!["p".into(), "a".repeat(64)]];
        let error = append(&metadata, &mut Vec::new()).expect_err("must reject");
        assert!(error.contains("unsupported prefix"));
    }
}
