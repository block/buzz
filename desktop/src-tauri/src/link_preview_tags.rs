use nostr::Tag;

pub fn append(preview_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for preview_tag in preview_tags {
        let valid = preview_tag.len() == 3
            && preview_tag.first().map(String::as_str) == Some("link-preview")
            && preview_tag.get(1).map(String::as_str) == Some("hide")
            && preview_tag
                .get(2)
                .and_then(|value| url::Url::parse(value).ok())
                .is_some_and(|url| url.scheme() == "https" && url.fragment().is_none());
        if !valid {
            return Err("invalid link-preview suppression tag".into());
        }
        let parts: Vec<&str> = preview_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid link-preview tag: {e}"))?);
    }
    Ok(())
}
