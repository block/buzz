import 'package:flutter/foundation.dart';

import 'mention_label_index.dart';

export 'mention_label_index.dart';

/// What continues a name directly after a label, as a pattern source: a
/// letter, mark or digit, or underscores leading to one, so `@Bobby` and
/// `@Bob_smith` hold no `Bob`. Anything else ends the name: punctuation, a
/// straight or curly apostrophe, a hyphen or a slash (`@Bob's`, `@Bob-bot`,
/// `@Bob/Alice`), and underscores that close Markdown emphasis (`_@Bob_`,
/// `__@Bob__`), which the composer's Italic button writes and sent messages
/// already hold. The composer's `mentionEndsAtBoundary`, the message scan
/// and the bubble's pill pattern all read this one rule.
const mentionNameContinuation = r'(?:[\p{L}\p{M}\p{N}]|_+[\p{L}\p{M}\p{N}])';

final _typedMentionEnd = RegExp(r'^(?:\|\||[\s,;.!?:)\]}*_]|$)');
final _markdownLinkLabel = RegExp(r'\[[^\]\n]*\]\(');

/// Whether a typed, unpicked name at `[start, end)` of [text] may notify by
/// name alone. A picked chip keeps its recipient before any suffix; a name
/// nobody picked must end as it always has, at whitespace, sentence
/// punctuation, a closing bracket or a Markdown delimiter, so the `@Alice` of
/// `@Alice@example.org` or `@Alice's` is prose. The label of a Markdown link
/// (`[@Alice](url)`) is link text, not a mention.
bool typedMentionNotifies(String text, int start, int end) {
  if (!_typedMentionEnd.hasMatch(text.substring(end))) return false;
  return !_markdownLinkLabel
      .allMatches(text)
      .any((link) => link.start < start && end < link.end - 1);
}

final _opensMention = RegExp(r'[\s(\[{*_]', unicode: true);
final _nameContinues = RegExp(mentionNameContinuation, unicode: true);
final _keyQualifierNext = RegExp(r' \([0-9a-f]{64}\)', caseSensitive: false);
final _keyQualified = RegExp(r' \([0-9a-f]{64}\)$', caseSensitive: false);
final _reservationNext = RegExp(
  ' (?:[2-9]|[1-9][0-9]+)(?!$mentionNameContinuation)',
  unicode: true,
);

/// How many candidate names [mentionOccurrences] has looked up, for tests
/// that check a scan's work does not grow with the roster.
@visibleForTesting
int mentionScanLookups = 0;

/// Longest literal ranges win, including labels containing another @ sign.
/// Recognition precedes eligibility: ambiguous labels still block shorter ones.
/// A label opens where a query does (portable mention rules, section 2): at
/// the start, after whitespace, `(`, `[` or `{`, or after a Markdown
/// delimiter, so a mention picked after a bracket keeps its recipient. It
/// ends at the composer's suffix boundary, [mentionNameContinuation].
///
/// No pattern is built from a name. Each `@` is found once, and the text
/// after it is looked up in a hash index at each length a label has, longest
/// first, as buzz-app's `typedContinuation` probes its name map. The cost
/// per `@` therefore depends on how many distinct lengths the labels have
/// and not on how many labels there are, and text with no `@` costs one
/// search for it. Names compare as a case-insensitive Unicode pattern
/// compares them ([foldMentionLabel]). [labels] are indexed on each call, so
/// a channel roster belongs in [known], built once and reused; a label in
/// [labels] wins over one in [known] that differs only by case.
List<({int start, int end, String label})> mentionOccurrences(
  String text,
  Iterable<String> labels, {
  MentionLabelIndex? known,
}) {
  final result = <({int start, int end, String label})>[];
  var at = text.indexOf('@');
  if (at < 0) return result;
  final own = MentionLabelIndex(labels);
  final lengths = known == null || known.lengths.isEmpty
      ? own.lengths
      : own.lengths.isEmpty
      ? known.lengths
      : ({...own.lengths, ...known.lengths}.toList()
          ..sort((a, b) => b.compareTo(a)));
  while (at >= 0) {
    var next = at + 1;
    final opens =
        at == 0 ||
        _opensMention.matchAsPrefix(text, at - 1) != null ||
        at >= 2 && text.startsWith('||', at - 2);
    if (opens) {
      for (final length in lengths) {
        final end = at + 1 + length;
        if (end > text.length) continue;
        // A label never ends inside a surrogate pair or against a name
        // character, and a key qualifier belongs to the literal even when no
        // candidate binds it: never fall back to a shorter, different
        // recipient.
        if (end < text.length &&
            (text.codeUnitAt(end) & 0xFC00) == 0xDC00 &&
            (text.codeUnitAt(end - 1) & 0xFC00) == 0xD800) {
          continue;
        }
        if (_nameContinues.matchAsPrefix(text, end) != null ||
            _keyQualifierNext.matchAsPrefix(text, end) != null) {
          continue;
        }
        mentionScanLookups++;
        final folded = foldMentionLabel(text.substring(at + 1, end));
        final label = own[folded] ?? known?[folded];
        // The same holds for a qualified label's reservation suffix.
        if (label == null ||
            _keyQualified.hasMatch(label) &&
                _reservationNext.matchAsPrefix(text, end) != null) {
          continue;
        }
        result.add((start: at, end: end, label: label));
        next = end;
        break;
      }
    }
    at = text.indexOf('@', next);
  }
  return result;
}

/// Bind tagged qualifiers and ordinary aliases, never historical namesakes.
/// Labels are keyed by [foldMentionLabel], as the text scan compares them,
/// so two names that fold alike are ambiguous.
Map<String, Set<String>> renderedMentionBindings(
  String content,
  Map<String, String> names, [
  Iterable<String> signedPubkeys = const [],
]) {
  final bindings = <String, Set<String>>{};
  void add(String label, String key) =>
      (bindings[foldMentionLabel(label)] ??= {}).add(key.toLowerCase());
  for (final entry in names.entries) {
    add(entry.value, entry.key);
    add(entry.value.split(RegExp(r'\s+')).first, entry.key);
  }
  final keys = signedPubkeys.map((key) => key.toLowerCase()).toSet();
  for (final match in RegExp(
    r'@([^@\r\n]+) \(([0-9a-f]{64})\)(?: ((?:[1-9][0-9]+|[2-9])))?',
    caseSensitive: false,
  ).allMatches(content)) {
    final key = match.group(2)!.toLowerCase();
    final label = foldMentionLabel(match.group(0)!.substring(1));
    if (!mentionOccurrences(content, [
      label,
    ]).any((range) => range.start == match.start)) {
      continue;
    }
    // Text can deny historical aliases without a profile, never authorize keys.
    bindings[label] = keys.contains(key) ? {key} : <String>{};
    // Recipient tags/current names cannot establish the historical plain owner.
    bindings[foldMentionLabel(match.group(1)!)] = <String>{};
  }
  return bindings;
}
