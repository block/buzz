/// Emoji search for the mobile picker and reaction tray.
///
/// Ported from `desktop/src/shared/lib/emojiSearch.ts`, which exists because
/// emoji-mart's own search is token-prefix based and so can't cross the `_` in
/// a shortcode — `pointup` never finds `point_up`. The tiers keep ordinary
/// queries ranked the way you'd expect while still surfacing looser matches
/// last: exact > prefix > substring > subsequence over the shortcode.
///
/// Mobile extends the desktop tiers with name/keyword matching, because the
/// tray replaces emoji-mart's picker (which searched names and keywords) rather
/// than just its `:shortcode` autocomplete. Shortcode hits still outrank
/// keyword hits, so typing a shortcode you know never buries it.
library;

import 'emoji_data.dart';

// Lower tier = stronger match.
const _tierExact = 0;
const _tierShortcodePrefix = 1;
const _tierKeywordPrefix = 2;
const _tierShortcodeSubstring = 3;
const _tierKeywordSubstring = 4;
const _tierSubsequence = 5;

/// Sentinel ranking worse than every real tier.
const _noMatch = 1 << 20;

final _separators = RegExp(r'[:_\s-]');
final _wordSplit = RegExp(r'[\s_-]+');

/// Lowercase and drop separators (`:`, `_`, `-`, whitespace) so matching is
/// insensitive to shortcode punctuation. `point_up` and `pointup` collapse to
/// the same normalized form.
///
/// Named for what it does rather than desktop's `normalizeShortcode`, because
/// `custom_emoji.dart` already exports a validating `normalizeShortcode` with
/// different semantics (it returns null for malformed input).
String collapseSeparators(String value) {
  return value.toLowerCase().replaceAll(_separators, '');
}

/// A ranked match. [score] is a within-tier tiebreak — lower is better.
class EmojiMatch {
  final int tier;
  final int score;

  const EmojiMatch({required this.tier, required this.score});
}

/// If [query] is a subsequence of [target] (all chars in order, gaps allowed),
/// return the span of the match as a tightness score — smaller is tighter.
/// Returns null when it isn't a subsequence.
int? _subsequenceSpan(String query, String target) {
  var first = -1;
  var last = -1;
  var qi = 0;
  for (var ti = 0; ti < target.length && qi < query.length; ti++) {
    if (target[ti] == query[qi]) {
      if (first == -1) first = ti;
      last = ti;
      qi++;
    }
  }
  if (qi < query.length) return null;
  return last - first;
}

/// Score [query] against a single [shortcode]. Returns null when there is no
/// match at any tier. Both sides are normalized first, so separators are
/// ignored. This is the desktop-identical half of the ranking.
EmojiMatch? scoreShortcodeMatch(String query, String shortcode) {
  final q = collapseSeparators(query);
  if (q.isEmpty) return null;
  final target = collapseSeparators(shortcode);
  if (target.isEmpty) return null;

  if (q == target) return const EmojiMatch(tier: _tierExact, score: 0);
  if (target.startsWith(q)) {
    return const EmojiMatch(tier: _tierShortcodePrefix, score: 0);
  }
  final index = target.indexOf(q);
  if (index != -1) {
    return EmojiMatch(tier: _tierShortcodeSubstring, score: index);
  }
  final span = _subsequenceSpan(q, target);
  if (span != null) return EmojiMatch(tier: _tierSubsequence, score: span);
  return null;
}

/// Score [query] against an entry's name and keywords, word by word. Used to
/// top up shortcode matching so a query like `smiling` finds emoji whose
/// shortcode doesn't contain it.
EmojiMatch? _scoreKeywordMatch(
  String query,
  String name,
  List<String> keywords,
) {
  final q = query.toLowerCase().trim();
  if (q.isEmpty) return null;

  var bestTier = _noMatch;
  var bestScore = 0;

  // Earlier words rank better: the name comes before the keywords, and within
  // each the dataset's own order is meaningful.
  void consider(String candidate, int wordIndex) {
    if (bestTier == _tierKeywordPrefix) return;
    final word = candidate.toLowerCase();
    if (word.isEmpty) return;
    final tier = word.startsWith(q)
        ? _tierKeywordPrefix
        : word.contains(q)
        ? _tierKeywordSubstring
        : _noMatch;
    if (tier < bestTier) {
      bestTier = tier;
      bestScore = wordIndex;
    }
  }

  var wordIndex = 0;
  for (final word in name.split(_wordSplit)) {
    consider(word, wordIndex++);
  }
  for (final keyword in keywords) {
    for (final word in keyword.split(_wordSplit)) {
      consider(word, wordIndex++);
    }
  }

  if (bestTier == _noMatch) return null;
  return EmojiMatch(tier: bestTier, score: bestScore);
}

class _Scored<T> {
  final T item;
  final int tier;
  final int score;
  final String code;

  const _Scored({
    required this.item,
    required this.tier,
    required this.score,
    required this.code,
  });
}

int _compare<T>(_Scored<T> a, _Scored<T> b) {
  if (a.tier != b.tier) return a.tier - b.tier;
  if (a.score != b.score) return a.score - b.score;
  // Shorter shortcode is the more specific match, then alphabetical for a
  // stable result.
  if (a.code.length != b.code.length) return a.code.length - b.code.length;
  return a.code.compareTo(b.code);
}

/// Rank [items] by how well their shortcode matches [query], best first, capped
/// at [limit]. Mirrors desktop's `rankByShortcode` — used for the custom-emoji
/// palette, which has no names or keywords.
List<T> rankByShortcode<T>(
  String query,
  List<T> items,
  String Function(T item) shortcodeOf, {
  int? limit,
}) {
  if (limit != null && limit <= 0) return const [];
  final scored = <_Scored<T>>[];
  for (final item in items) {
    final code = shortcodeOf(item);
    final match = scoreShortcodeMatch(query, code);
    if (match == null) continue;
    scored.add(
      _Scored(item: item, tier: match.tier, score: match.score, code: code),
    );
  }
  scored.sort(_compare);
  final ranked = scored.map((entry) => entry.item).toList();
  if (limit == null || ranked.length <= limit) return ranked;
  return ranked.sublist(0, limit);
}

/// Rank standard emoji by shortcode, name, and keywords, best first.
List<EmojiEntry> searchEmoji(
  String query,
  List<EmojiEntry> entries, {
  int? limit,
}) {
  if (limit != null && limit <= 0) return const [];
  final scored = <_Scored<EmojiEntry>>[];
  for (final entry in entries) {
    final shortcode = scoreShortcodeMatch(query, entry.id);
    final keyword = _scoreKeywordMatch(query, entry.name, entry.keywords);
    final match = switch ((shortcode, keyword)) {
      (null, null) => null,
      (final EmojiMatch only, null) => only,
      (null, final EmojiMatch only) => only,
      (final EmojiMatch a, final EmojiMatch b) => a.tier <= b.tier ? a : b,
    };
    if (match == null) continue;
    scored.add(
      _Scored(
        item: entry,
        tier: match.tier,
        score: match.score,
        code: entry.id,
      ),
    );
  }
  scored.sort(_compare);
  final ranked = scored.map((entry) => entry.item).toList();
  if (limit == null || ranked.length <= limit) return ranked;
  return ranked.sublist(0, limit);
}
