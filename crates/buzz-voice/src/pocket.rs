use std::collections::HashMap;

/// Tight `max_frames` for short, padded prompts to bound Pocket TTS runaway.
pub const SHORT_PROMPT_MAX_FRAMES: i32 = 100;

/// Word-count threshold (inclusive) below which Pocket's upstream prompt prep pads input.
pub const SHORT_PROMPT_WORD_THRESHOLD: usize = 4;

/// Number of leading spaces upstream Pocket prep applies to short prompts.
pub const SHORT_PROMPT_PAD_SPACES: usize = 8;

/// Result of [`prepare_pocket_prompt`]: a synthesizer-ready prompt plus the
/// per-call generation overrides derived from the original text.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedPrompt {
    /// Text to hand to the Pocket TTS engine.
    pub text: String,
    /// Value to pass via `GenerationConfig.extra["max_frames"]`, or `None` to
    /// keep the upstream default.
    pub max_frames: Option<i32>,
}

pub fn prepare_pocket_prompt(input: &str) -> Option<PreparedPrompt> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut cleaned = String::with_capacity(trimmed.len());
    let mut last_was_space = false;
    for ch in trimmed.chars() {
        let is_ws = ch.is_whitespace();
        if is_ws {
            if !last_was_space {
                cleaned.push(' ');
            }
            last_was_space = true;
        } else {
            cleaned.push(ch);
            last_was_space = false;
        }
    }

    let first = cleaned.chars().next().expect("cleaned non-empty above");
    if first.is_lowercase() {
        let upper: String = first.to_uppercase().collect();
        let mut iter = cleaned.chars();
        iter.next();
        cleaned = upper + iter.as_str();
    }

    let last = cleaned
        .chars()
        .next_back()
        .expect("cleaned non-empty above");
    if !matches!(last, '.' | '!' | '?' | ';' | ':' | ',') {
        cleaned.push('.');
    }

    let word_count = cleaned.split_whitespace().count();

    let (final_text, max_frames) = if word_count <= SHORT_PROMPT_WORD_THRESHOLD {
        let mut padded = String::with_capacity(cleaned.len() + SHORT_PROMPT_PAD_SPACES);
        for _ in 0..SHORT_PROMPT_PAD_SPACES {
            padded.push(' ');
        }
        padded.push_str(&cleaned);
        (padded, Some(SHORT_PROMPT_MAX_FRAMES))
    } else {
        (cleaned, None)
    };

    Some(PreparedPrompt {
        text: final_text,
        max_frames,
    })
}

pub fn build_generation_extra(
    prepared: &PreparedPrompt,
) -> Option<HashMap<String, serde_json::Value>> {
    prepared.max_frames.map(|max_frames| {
        let mut extra = HashMap::with_capacity(1);
        extra.insert(
            "max_frames".to_string(),
            serde_json::Value::from(max_frames),
        );
        extra
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHERPA_ONNX_MAX_FRAMES_DEFAULT: i32 = 500;
    const SHERPA_ONNX_FRAMES_AFTER_EOS_DEFAULT: i32 = 3;

    #[test]
    fn prepare_prompt_returns_none_for_empty_input() {
        assert!(prepare_pocket_prompt("").is_none());
        assert!(prepare_pocket_prompt("   ").is_none());
        assert!(prepare_pocket_prompt("\n\t  ").is_none());
    }

    #[test]
    fn prepare_prompt_pads_and_capitalizes_one_word() {
        let out = prepare_pocket_prompt("yep").expect("non-empty");
        assert_eq!(out.text, "        Yep.");
        assert_eq!(out.max_frames, Some(SHORT_PROMPT_MAX_FRAMES));
    }

    #[test]
    fn prepare_prompt_preserves_existing_punctuation() {
        let out = prepare_pocket_prompt("yes!").expect("non-empty");
        assert_eq!(out.text, "        Yes!");
        let out = prepare_pocket_prompt("really?").expect("non-empty");
        assert_eq!(out.text, "        Really?");
    }

    #[test]
    fn prepare_prompt_threshold_is_inclusive_at_four_words() {
        let four = prepare_pocket_prompt("one two three four").expect("non-empty");
        assert_eq!(four.text, "        One two three four.");
        assert_eq!(four.max_frames, Some(SHORT_PROMPT_MAX_FRAMES));
    }

    #[test]
    fn prepare_prompt_does_not_pad_long_text() {
        let five = prepare_pocket_prompt("one two three four five").expect("non-empty");
        assert_eq!(five.text, "One two three four five.");
        assert_eq!(five.max_frames, None);
    }

    #[test]
    fn build_extra_sets_only_max_frames_for_short_prompts() {
        let prepared = prepare_pocket_prompt("yep").expect("non-empty");
        let extra = build_generation_extra(&prepared).expect("short prompt extra");
        assert_eq!(extra.get("max_frames"), Some(&serde_json::Value::from(100)));
        assert!(!extra.contains_key("frames_after_eos"));
    }

    #[test]
    fn build_extra_uses_defaults_for_long_prompts() {
        let prepared = prepare_pocket_prompt("Yep, I can hear you.").expect("non-empty");
        assert!(build_generation_extra(&prepared).is_none());
        assert_eq!(SHERPA_ONNX_FRAMES_AFTER_EOS_DEFAULT, 3);
        assert_eq!(SHERPA_ONNX_MAX_FRAMES_DEFAULT, 500);
    }
}
