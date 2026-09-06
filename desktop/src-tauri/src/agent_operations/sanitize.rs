pub(crate) fn sanitize_name(input: &str) -> String {
    let mut output = String::new();
    for character in input.chars() {
        if output.chars().count() >= 80 {
            break;
        }
        let unsafe_character = character.is_control()
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '@'
                    | '|'
                    | '`'
                    | '*'
                    | '_'
                    | '['
                    | ']'
                    | '<'
                    | '>'
                    | '#'
            );
        output.push(if unsafe_character { ' ' } else { character });
    }
    let collapsed = output.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "Unnamed agent".to_string()
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn syn79_alert_names_neutralize_mentions_markdown_and_controls() {
        assert_eq!(super::sanitize_name("@ops | **boom**\n`x`"), "ops boom x");
    }
}
