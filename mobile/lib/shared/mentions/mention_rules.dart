import 'package:flutter/foundation.dart';

/// One key the user can mention. The fields are the inputs of the portable
/// mention rules (buzz-app `src/bundled/mentions/README.md`, version 1).
@immutable
class MentionChoice {
  /// Lowercase 64-hex public key.
  final String pubkey;

  /// The identity's own name.
  final String name;

  /// The name shown in the chooser. It can be qualified or a key fallback.
  final String label;

  /// The real names known for this key. Empty when no name is known.
  final List<String> aliases;
  final bool member;
  final bool agent;

  /// Ownership is a display hint from profile metadata, not authorization.
  final bool owned;
  final bool managed;

  const MentionChoice({
    required this.pubkey,
    required this.name,
    required this.label,
    required this.aliases,
    required this.member,
    this.agent = false,
    this.owned = false,
    this.managed = false,
  });
}

String _normalized(String text) => text.trim().toLowerCase();

final _whitespace = RegExp(r'\s+', unicode: true);

/// Tier 0 to 3 from section 3, or null for no match.
int? _nameMatch(String text, String needle) {
  if (needle.isEmpty) return 0;
  final name = _normalized(text);
  if (name == needle) return 0;
  if (name.startsWith(needle)) return 1;
  final words = name.split(_whitespace);
  if (words.contains(needle)) return 2;
  if (words.any((word) => word.startsWith(needle))) return 3;
  return null;
}

int? _min(Iterable<int?> values) {
  int? best;
  for (final value in values) {
    if (value != null && (best == null || value < best)) best = value;
  }
  return best;
}

int? _baseMatch(MentionChoice choice, String needle) =>
    _min(choice.aliases.map((name) => _nameMatch(name, needle)));

/// A choice's match (section 3): the label tier, else 4 plus the best alias
/// tier. Null means the choice does not match. Keys are never searchable.
@visibleForTesting
int? mentionMatch(MentionChoice choice, String query) {
  final needle = _normalized(query);
  if (needle.isEmpty) return 0;
  // No known name: the label is only a key fallback.
  if (choice.aliases.isEmpty) return null;
  final visible = _nameMatch(choice.label, needle);
  if (visible != null) return visible;
  final base = _baseMatch(choice, needle);
  return base == null ? null : 4 + base;
}

int _compare(String a, String b) => a.compareTo(b);

/// Section 4 order. Every rule is a key of one choice, so the result is the
/// same for any input order. Agents that share a name form one block at its
/// lowest label; recency, managed and presence order only that block.
List<MentionChoice> rankMentions(
  Iterable<MentionChoice> choices,
  String query, {
  Map<String, int> history = const {},
  String Function(String pubkey)? presence,
}) {
  final needle = _normalized(query);
  final keyed = <_Keyed>[];
  for (final choice in choices) {
    final match = mentionMatch(choice, query);
    if (match == null) continue;
    final status = presence?.call(choice.pubkey);
    keyed.add(
      _Keyed(
        choice: choice,
        tier: [
          choice.member ? 0 : 1,
          match,
          // No aliases: after every named choice, even for the empty query.
          _baseMatch(choice, needle) ?? 1 << 30,
          choice.agent && choice.owned ? 0 : 1,
        ],
        label: _normalized(choice.label),
        blockName: choice.agent ? _normalized(choice.name) : null,
        recent: history[choice.pubkey] ?? 0,
        online: switch (status) {
          'online' => 0,
          'away' => 1,
          _ => 2,
        },
      ),
    );
  }
  final firstLabel = <String, String>{};
  for (final k in keyed) {
    final label = firstLabel[k.block];
    if (label == null || _compare(k.label, label) < 0) {
      firstLabel[k.block] = k.label;
    }
  }
  keyed.sort((a, b) {
    for (var i = 0; i < a.tier.length; i++) {
      final d = a.tier[i] - b.tier[i];
      if (d != 0) return d;
    }
    var d = _compare(firstLabel[a.block]!, firstLabel[b.block]!);
    if (d != 0) return d;
    // People never join a block. On an equal block label they sort first.
    d = (a.blockName == null ? 0 : 1) - (b.blockName == null ? 0 : 1);
    if (d != 0) return d;
    d = a.blockName == null
        ? _compare(a.choice.pubkey, b.choice.pubkey)
        : _compare(a.blockName!, b.blockName!);
    if (d != 0) return d;
    d = b.recent - a.recent;
    if (d != 0) return d;
    d = (b.choice.managed ? 1 : 0) - (a.choice.managed ? 1 : 0);
    if (d != 0) return d;
    d = a.online - b.online;
    if (d != 0) return d;
    d = _compare(a.label, b.label);
    if (d != 0) return d;
    return _compare(a.choice.pubkey, b.choice.pubkey);
  });
  return [for (final k in keyed) k.choice];
}

class _Keyed {
  final MentionChoice choice;
  final List<int> tier;
  final String label;

  /// Normalized name for an agent, null for a person.
  final String? blockName;
  final int recent;
  final int online;

  _Keyed({
    required this.choice,
    required this.tier,
    required this.label,
    required this.blockName,
    required this.recent,
    required this.online,
  });

  late final String block = blockName == null
      ? '0${choice.pubkey}'
      : '1${tier.join(',')}\u0000$blockName';
}

/// Section 5: the key Space selects, or null. Checked against the full,
/// uncapped choice set.
String? exactMention(Iterable<MentionChoice> choices, String query) {
  final needle = _normalized(query);
  if (needle.isEmpty) return null;
  String? found;
  var count = 0;
  for (final choice in choices) {
    final names = [...choice.aliases, choice.label].map(_normalized);
    if (names.any((name) => name.startsWith('$needle '))) return null;
    if (choice.aliases.isNotEmpty && names.contains(needle)) {
      count++;
      found = choice.pubkey;
    }
  }
  return count == 1 ? found : null;
}

/// An open `@` query: `[start, end)` covers `@` and the query text.
typedef MentionQuery = ({int start, int end, String query});

final _queryPattern = RegExp(r'(?:^|[\s(\[{])@([^@\n\r\t]*)$', unicode: true);
final _boundary = RegExp(r'[\s(\[{]', unicode: true);

/// Section 2 syntax only. Multi-word queries are admitted separately by
/// [matchesMentionQuery] against current names.
MentionQuery? findMentionQuery(String text, int caret) {
  if (caret < 0 || caret > text.length) return null;
  final before = text.substring(caret > 160 ? caret - 160 : 0, caret);
  final match = _queryPattern.firstMatch(before);
  if (match == null) return null;
  final query = match.group(1) ?? '';
  final start = caret - query.length - 1;
  // A bounded slice that begins inside a word is not a boundary.
  if (start > 0 && !_boundary.hasMatch(text[start - 1])) return null;
  return (start: start, end: caret, query: query);
}

/// Section 2 multi-word rule: a query with a space stays open only while it
/// is still the start of a known name. A complete name plus a trailing space
/// is prose.
bool matchesMentionQuery(String query, Iterable<String> names) {
  if (!query.contains(' ')) return true;
  final lower = query.toLowerCase();
  if (lower.endsWith(' ') &&
      names.any((name) => name.toLowerCase() == lower.trimRight())) {
    return false;
  }
  return names.any((name) => name.toLowerCase().startsWith(lower));
}
