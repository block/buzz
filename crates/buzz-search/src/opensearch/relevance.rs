//! How the index analyzes message text and how a search ranks it.
//!
//! Every relevance choice lives here: the analyzers, the text fields they
//! fill, the match query and the age rescore. The rest of [`super`] only
//! filters and pages. The reasons the code can't show:
//!
//! - **Letters and digits stay together.** `word_delimiter_graph` splits at
//!   `.`, `_`, `-`, `:` and case changes but not between letters and digits,
//!   so a hex id stays one token instead of dozens of fragments. The query
//!   is split the same way, so part of an identifier (`snake_case` of
//!   `snake_case_name`, `OutboxBatch` of `SearchOutboxBatch`) finds it. On
//!   `content` the split parts are a phrase, so `foo_bar` ranks itself, then
//!   `foo bar`. The stem and prefix fields keep no positions, so there the
//!   parts match anywhere: a message holding `foo` and `bar` apart is still
//!   found, last.
//! - **Typos only rank.** A word matched only with a typo scores a third
//!   of an exact match, so relevance order keeps it below. Newest order has
//!   no such defence: for a short word most matches are other words one
//!   letter away (`log` and `lag`), so it matches no typos.
//! - **Long tokens get no prefixes.** Tokens over [`LONG_TOKEN`] characters
//!   (`nostr:` URIs, keys, hashes) are not typed a letter at a time, and
//!   each would add dozens of prefixes.
//! - **Phrase and proximity are a rescore.** They read the positions of
//!   every message holding all the words. In the main query that cost 55 ms
//!   for two common words on a million messages, against 22 ms over the top
//!   [`RESCORE_WINDOW`] hits, and it grows with the history.
//! - **Age is a rescore too**: `0.2 + 0.8 × 0.5^(age / 30 days)`, applied to
//!   the top [`RESCORE_WINDOW`] hits per shard. Matching can still skip
//!   documents that cannot reach the top, so cost does not grow with every
//!   match of a common word. The trade-off: recency cannot lift a weak match
//!   from outside the window.
//! - **Serverless.** Built-in analysis only (no ICU plugin), and no scripts
//!   are used. `word_delimiter_graph`, `edge_ngram`, `length`, `truncate`,
//!   the `light_english` stemmer and `rescore` with a `function_score`
//!   decay were checked on a NextGen Serverless collection, where an inline
//!   Painless `script_score` also ran.

use serde_json::{json, Value};

use crate::SearchMode;

/// Shortest prefix indexed for prefix mode.
const MIN_PREFIX: u32 = 2;
/// Longest prefix indexed: a longer partial word matches through its first
/// [`MAX_PREFIX`] characters. Ten characters rather than twenty makes the
/// prefix field a quarter smaller (1.03 to 0.77 KB a message on a million
/// messages, 2.7 times the whole-word field), at the same search speed.
const MAX_PREFIX: u32 = 10;
/// Tokens longer than this get no prefixes.
const LONG_TOKEN: u32 = 24;

/// How many of the query words a message must match: all of one or two,
/// three quarters (rounded down) of more.
const MIN_WORDS_MATCHED: &str = "2<75%";
/// Query words past this many are ignored, which bounds a query's clauses.
const MAX_WORDS: usize = 32;
/// Weight of a word matched only through its stem.
const STEM_WEIGHT: f64 = 0.4;
/// Weight of a word matched only with a typo.
const TYPO_WEIGHT: f64 = 0.3;
/// Weight of a partial last word matched as a prefix.
const PREFIX_WEIGHT: f64 = 0.5;
/// Bonus weight of the query found as an exact phrase. An exact phrase is
/// also within [`PROXIMITY_SLOP`], so it earns both bonuses.
const PHRASE_WEIGHT: f64 = 2.0;
/// Bonus weight of the query words found within [`PROXIMITY_SLOP`] moves of
/// each other.
const PROXIMITY_WEIGHT: f64 = 1.0;
const PROXIMITY_SLOP: u32 = 4;

/// How many of the best hits by Q the age factor reorders, and so how far
/// relevance results go: past the window a hit would keep Q alone and
/// interleave with the window's, so pages would repeat or skip hits. It
/// covers the relay's search scan, which pages through up to
/// [`buzz_db::DEFAULT_MAX_PAGE_LIMIT`] candidates. On a million messages,
/// 1,000 rather than 500 added 3 ms to a one-word search (29 to 32 ms) and
/// nothing measurable to longer ones, whose matching costs far more.
pub(super) const RESCORE_WINDOW: i64 = 1_000;
const _: () = assert!(RESCORE_WINDOW >= buzz_db::DEFAULT_MAX_PAGE_LIMIT);
/// T never falls below this, so old exact matches survive.
const AGE_FLOOR: f64 = 0.2;
/// Age at which the decaying part of T has halved.
const AGE_HALF_LIFE_DAYS: f64 = 30.0;

/// The index's `analysis` settings.
pub(super) fn analysis() -> Value {
    json!({
        "filter": {
            "word_parts": {
                "type": "word_delimiter_graph",
                "preserve_original": true,
                "split_on_numerics": false
            },
            "short_tokens": { "type": "length", "max": LONG_TOKEN },
            "prefixes": { "type": "edge_ngram", "min_gram": MIN_PREFIX, "max_gram": MAX_PREFIX },
            "longest_prefix": { "type": "truncate", "length": MAX_PREFIX },
            "light_stem": { "type": "stemmer", "language": "light_english" }
        },
        "analyzer": {
            "message": analyzer(&["word_parts", "lowercase", "asciifolding"]),
            "message_search": analyzer(&["word_parts", "lowercase", "asciifolding"]),
            "message_stem": analyzer(&["word_parts", "lowercase", "asciifolding", "light_stem"]),
            "message_stem_search": analyzer(&[
                "word_parts", "lowercase", "asciifolding", "light_stem"
            ]),
            "message_prefix": analyzer(&[
                "word_parts", "lowercase", "asciifolding", "short_tokens", "prefixes"
            ]),
            "message_prefix_search": analyzer(&[
                "word_parts", "lowercase", "asciifolding", "longest_prefix"
            ])
        }
    })
}

fn analyzer(filters: &[&str]) -> Value {
    json!({ "type": "custom", "tokenizer": "standard", "filter": filters })
}

/// The mapping of `content`, the message text.
pub(super) fn content_mapping() -> Value {
    json!({
        "type": "text",
        "analyzer": "message",
        "search_analyzer": "message_search",
        "fields": {
            "stem": {
                "type": "text",
                "analyzer": "message_stem",
                "search_analyzer": "message_stem_search",
                "index_options": "freqs"
            },
            "prefix": prefix_mapping()
        }
    })
}

/// The prefix field: edge n-grams, searched with whole query tokens.
fn prefix_mapping() -> Value {
    json!({
        "type": "text",
        "analyzer": "message_prefix",
        "search_analyzer": "message_prefix_search",
        "index_options": "freqs"
    })
}

/// `word` as a prefix of an indexed word.
fn prefix_query(word: &str) -> Value {
    json!({ "match": { "content.prefix": {
        "query": word, "operator": "and", "boost": PREFIX_WEIGHT
    } } })
}

/// `word` with up to `AUTO` edits (none for one or two characters, one up to
/// five, then two), its first letter exact.
fn typo_clause(word: &str) -> Value {
    json!({ "match": { "content": {
        "query": word,
        "operator": "and",
        "fuzziness": "AUTO",
        "prefix_length": 1,
        "boost": TYPO_WEIGHT
    } } })
}

/// A search's text: what matches, and the phrase bonus for a
/// [`rescore`].
pub(super) struct TextQuery {
    /// The words: which messages match, and their Q before any bonus.
    pub(super) matched: Value,
    /// The phrase and proximity bonus, for queries of two words or more.
    bonus: Option<Value>,
}

/// The text query for `text` in `mode`, or `None` if it has no words.
/// `typos` says whether a word may match with a typo.
pub(super) fn text_query(text: &str, mode: SearchMode, typos: bool) -> Option<TextQuery> {
    // A punctuation-only word analyzes to nothing and would count against
    // the minimum without ever matching.
    let words: Vec<&str> = text
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .take(MAX_WORDS)
        .collect();
    let (last, completed) = words.split_last()?;
    let mut clauses: Vec<Value> = completed
        .iter()
        .map(|word| word_clause(word, typos))
        .collect();
    clauses.push(match mode {
        SearchMode::FullText => word_clause(last, typos),
        SearchMode::Prefix => json!({ "dis_max": { "queries": [
            exact_clause(last),
            stem_clause(last),
            prefix_query(last),
        ] } }),
    });
    let matched = json!({ "bool": {
        "should": clauses,
        "minimum_should_match": MIN_WORDS_MATCHED
    } });
    if words.len() < 2 {
        return Some(TextQuery {
            matched,
            bonus: None,
        });
    }
    let phrase = words.join(" ");
    let phrase_query = match mode {
        SearchMode::FullText => "match_phrase",
        SearchMode::Prefix => "match_phrase_prefix",
    };
    let bonus = |slop: u32, boost: f64| json!({ phrase_query: { "content": { "query": phrase, "slop": slop, "boost": boost } } });
    Some(TextQuery {
        matched,
        bonus: Some(json!({ "bool": {
            "should": [bonus(0, PHRASE_WEIGHT), bonus(PROXIMITY_SLOP, PROXIMITY_WEIGHT)]
        } })),
    })
}

/// One query word, matched exactly, by stem or (with `typos`) with a typo,
/// scored by the best of them.
fn word_clause(word: &str, typos: bool) -> Value {
    let mut queries = vec![exact_clause(word), stem_clause(word)];
    if typos {
        queries.push(typo_clause(word));
    }
    json!({ "dis_max": { "queries": queries } })
}

fn exact_clause(word: &str) -> Value {
    json!({ "match": { "content": { "query": word, "operator": "and" } } })
}

fn stem_clause(word: &str) -> Value {
    json!({ "match": { "content.stem": {
        "query": word, "operator": "and", "boost": STEM_WEIGHT
    } } })
}

/// The rescores of a relevance search over the top [`RESCORE_WINDOW`] hits:
/// add `text`'s phrase bonus to their Q, then multiply it by the age factor
/// at `now` (Unix seconds); see [`age_factor`].
pub(super) fn rescore(text: &TextQuery, now: i64) -> Value {
    let bonus = text.bonus.as_ref().map(|bonus| {
        json!({
            "window_size": RESCORE_WINDOW,
            "query": {
                "rescore_query": bonus,
                "query_weight": 1.0,
                "rescore_query_weight": 1.0,
                "score_mode": "total"
            }
        })
    });
    Value::Array(bonus.into_iter().chain([age_rescore(now)]).collect())
}

fn age_rescore(now: i64) -> Value {
    json!({
        "window_size": RESCORE_WINDOW,
        "query": {
            "rescore_query": { "function_score": {
                "query": { "match_all": {} },
                "functions": [
                    {
                        "exp": { "created_at": {
                            "origin": now.to_string(),
                            "scale": format!("{AGE_HALF_LIFE_DAYS}d"),
                            "decay": 0.5
                        } },
                        "weight": 1.0 - AGE_FLOOR
                    },
                    { "weight": AGE_FLOOR }
                ],
                "score_mode": "sum",
                "boost_mode": "replace"
            } },
            "query_weight": 1.0,
            "rescore_query_weight": 1.0,
            "score_mode": "multiply"
        }
    })
}

/// T for a message created at `created_at`, at `now` (both Unix seconds):
/// what [`rescore`] multiplies by. A message from the future (clock
/// skew) decays as if as old.
pub(super) fn age_factor(created_at: i64, now: i64) -> f64 {
    let age_days = (now - created_at).unsigned_abs() as f64 / 86_400.0;
    AGE_FLOOR + (1.0 - AGE_FLOOR) * 0.5_f64.powf(age_days / AGE_HALF_LIFE_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_factor_halves_its_decaying_part_every_thirty_days() {
        let now = 1_800_000_000;
        let day = 86_400;
        for (age, expected) in [(0, 1.0), (30 * day, 0.6), (60 * day, 0.4)] {
            assert!((age_factor(now - age, now) - expected).abs() < 1e-9);
        }
        assert!((age_factor(now - 3650 * day, now) - AGE_FLOOR).abs() < 1e-9);
    }

    #[test]
    fn punctuation_only_queries_have_no_words() {
        assert!(text_query(" ' : & | ", SearchMode::FullText, true).is_none());
        assert!(text_query("alpha ' bet", SearchMode::Prefix, true).is_some());
    }
}
