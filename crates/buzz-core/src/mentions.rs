//! Display-name mention resolution shared by message producers.

use std::collections::{HashMap, HashSet};

/// Resolves `@Name` mentions to uniquely named member pubkeys.
///
/// Matching is case-insensitive and anchored at the start of the text or after
/// whitespace or `(`. Longer names take precedence, a name cannot match inside
/// a longer word, and ambiguous display names resolve to no pubkey. Returned
/// pubkeys are deduplicated in first-appearance order.
pub fn resolve_mention_pubkeys(text: &str, members: &[(String, String)]) -> Vec<String> {
    // Name -> pubkey, folding case. A name that maps to more than one distinct
    // pubkey is ambiguous and must wake no one.
    let mut by_name: HashMap<String, Option<String>> = HashMap::new();
    for (name, pubkey) in members {
        if name.trim().is_empty() {
            continue;
        }
        by_name
            .entry(name.to_lowercase())
            .and_modify(|slot| {
                if slot.as_deref() != Some(pubkey.as_str()) {
                    *slot = None;
                }
            })
            .or_insert_with(|| Some(pubkey.clone()));
    }

    // Match longest names first so a longer name consumes its span before a
    // shorter substring name can claim part of it.
    let mut names: Vec<&(String, String)> = members.iter().collect();
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.chars().count()));

    let chars: Vec<char> = text.chars().collect();
    let mut consumed = vec![false; chars.len()];

    // Fold on the fly because lowercasing can change character count. Tracking
    // consumed original characters keeps boundary accounting Unicode-safe.
    let match_name_len = |start: usize, folded_name: &[char]| -> Option<usize> {
        let mut char_index = start;
        let mut name_index = 0;
        while name_index < folded_name.len() {
            let character = *chars.get(char_index)?;
            for folded_character in character.to_lowercase() {
                if folded_name.get(name_index) != Some(&folded_character) {
                    return None;
                }
                name_index += 1;
            }
            char_index += 1;
        }
        Some(char_index - start)
    };

    let is_left_boundary =
        |index: usize| index == 0 || chars[index - 1].is_whitespace() || chars[index - 1] == '(';
    let extends_name = |character: char| character.is_alphanumeric() || character == '_';

    let mut hits: Vec<(usize, String)> = Vec::new();
    for (name, _) in &names {
        let folded_name: Vec<char> = name.to_lowercase().chars().collect();
        if folded_name.is_empty() {
            continue;
        }

        let mut at = 0;
        while at < chars.len() {
            let name_len = (chars[at] == '@' && is_left_boundary(at) && !consumed[at])
                .then(|| match_name_len(at + 1, &folded_name))
                .flatten()
                .filter(|&length| {
                    chars[at + 1 + length..]
                        .first()
                        .is_none_or(|&character| !extends_name(character))
                });

            if let Some(name_len) = name_len {
                let span = 1 + name_len;
                if let Some(Some(pubkey)) = by_name.get(&name.to_lowercase()) {
                    hits.push((at, pubkey.clone()));
                }
                for slot in consumed.iter_mut().skip(at).take(span) {
                    *slot = true;
                }
                at += span;
            } else {
                at += 1;
            }
        }
    }

    hits.sort_by_key(|(at, _)| *at);
    let mut seen = HashSet::new();
    hits.into_iter()
        .filter_map(|(_, pubkey)| seen.insert(pubkey.clone()).then_some(pubkey))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, pubkey: &str) -> (String, String) {
        (name.to_string(), pubkey.to_string())
    }

    fn pubkey(nibble: char) -> String {
        std::iter::repeat_n(nibble, 64).collect()
    }

    #[test]
    fn resolves_exact_member_name() {
        let members = vec![member("Robby", &pubkey('a'))];
        assert_eq!(
            resolve_mention_pubkeys("heads up @Robby — please take a look", &members),
            vec![pubkey('a')]
        );
    }

    #[test]
    fn matches_case_insensitively() {
        let members = vec![member("Robby", &pubkey('a'))];
        assert_eq!(
            resolve_mention_pubkeys("ping @robby", &members),
            vec![pubkey('a')]
        );
    }

    #[test]
    fn ignores_non_member_and_bare_at() {
        let members = vec![member("Robby", &pubkey('a'))];
        assert!(resolve_mention_pubkeys("hey @Stranger and @", &members).is_empty());
    }

    #[test]
    fn greedy_longest_binds_full_name_not_prefix() {
        let members = vec![
            member("Will", &pubkey('1')),
            member("Will Pfleger", &pubkey('2')),
        ];
        assert_eq!(
            resolve_mention_pubkeys("cc @Will Pfleger on this", &members),
            vec![pubkey('2')]
        );
        assert_eq!(
            resolve_mention_pubkeys("cc @Will on this", &members),
            vec![pubkey('1')]
        );
    }

    #[test]
    fn at_mid_token_does_not_match() {
        let members = vec![member("Robby", &pubkey('a'))];
        assert!(resolve_mention_pubkeys("alice@Robby", &members).is_empty());
    }

    #[test]
    fn prefix_member_does_not_match_inside_longer_word() {
        let members = vec![member("Sam", &pubkey('3'))];
        assert!(resolve_mention_pubkeys("hi @Sami", &members).is_empty());
    }

    #[test]
    fn name_with_spaces_and_punctuation() {
        let members = vec![member("Lep (Subagent)", &pubkey('4'))];
        assert_eq!(
            resolve_mention_pubkeys("@Lep (Subagent) take it", &members),
            vec![pubkey('4')]
        );
    }

    #[test]
    fn em_dash_terminates_name() {
        let members = vec![member("Robby", &pubkey('a'))];
        assert_eq!(
            resolve_mention_pubkeys("@Robby—please look", &members),
            vec![pubkey('a')]
        );
    }

    #[test]
    fn non_ascii_member_name() {
        let members = vec![member("Zoë", &pubkey('5'))];
        assert_eq!(
            resolve_mention_pubkeys("welcome @Zoë!", &members),
            vec![pubkey('5')]
        );
    }

    #[test]
    fn lowercase_expansion_does_not_shift_later_mentions() {
        let members = vec![member("İ", &pubkey('c')), member("Robby", &pubkey('a'))];
        assert_eq!(
            resolve_mention_pubkeys("@İ @Robby", &members),
            vec![pubkey('c'), pubkey('a')]
        );
    }

    #[test]
    fn sharp_s_matches_case_insensitively() {
        let members = vec![member("ẞ", &pubkey('d')), member("Max", &pubkey('b'))];
        assert_eq!(
            resolve_mention_pubkeys("@ẞ and @Max", &members),
            vec![pubkey('d'), pubkey('b')]
        );
    }

    #[test]
    fn combining_mark_in_name_matches() {
        let members = vec![member("Jos\u{0065}\u{0301}", &pubkey('4'))];
        assert_eq!(
            resolve_mention_pubkeys("hi @Jos\u{0065}\u{0301}!", &members),
            vec![pubkey('4')]
        );
    }

    #[test]
    fn expanding_name_at_trailing_boundary() {
        let members = vec![member("İ", &pubkey('5'))];
        assert_eq!(resolve_mention_pubkeys("@İ", &members), vec![pubkey('5')]);
        assert!(resolve_mention_pubkeys("@İx", &members).is_empty());
    }

    #[test]
    fn back_to_back_at_is_one_mention() {
        let members = vec![member("İ", &pubkey('5')), member("Robby", &pubkey('a'))];
        assert_eq!(
            resolve_mention_pubkeys("@İ@Robby", &members),
            vec![pubkey('5')]
        );
        let ascii = vec![member("Sam", &pubkey('6')), member("Robby", &pubkey('a'))];
        assert_eq!(
            resolve_mention_pubkeys("@Sam@Robby", &ascii),
            vec![pubkey('6')]
        );
        assert_eq!(
            resolve_mention_pubkeys("@İ @Robby", &members),
            vec![pubkey('5'), pubkey('a')]
        );
    }

    #[test]
    fn ambiguous_name_wakes_no_one() {
        let members = vec![
            member("Fizz", &pubkey('6')),
            member("Fizz", &pubkey('7')),
            member("Fizz", &pubkey('8')),
        ];
        assert!(resolve_mention_pubkeys("@Fizz status?", &members).is_empty());
    }

    #[test]
    fn duplicate_name_same_pubkey_is_not_ambiguous() {
        let members = vec![member("Fizz", &pubkey('6')), member("Fizz", &pubkey('6'))];
        assert_eq!(
            resolve_mention_pubkeys("@Fizz go", &members),
            vec![pubkey('6')]
        );
    }

    #[test]
    fn dedupes_repeated_mentions_in_first_appearance_order() {
        let members = vec![member("Robby", &pubkey('a')), member("Max", &pubkey('b'))];
        assert_eq!(
            resolve_mention_pubkeys("@Max then @Robby then @Max again", &members),
            vec![pubkey('b'), pubkey('a')]
        );
    }
}
