import 'package:flutter/foundation.dart';

import '../utils/string_utils.dart';
import 'name_match.dart';

/// One key the user can mention. The fields are the inputs of the portable
/// mention rules (buzz-app `src/bundled/mentions/README.md`, version 4).
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

  /// The other community this agent of the viewer's comes from, or null for
  /// a choice of this community (section 4, rule 2).
  final String? elsewhere;

  const MentionChoice({
    required this.pubkey,
    required this.name,
    required this.label,
    required this.aliases,
    required this.member,
    this.agent = false,
    this.owned = false,
    this.managed = false,
    this.elsewhere,
  });

  /// Every name of this choice: its aliases and its label.
  List<String> get _names => [...aliases, label];
}

List<int> _runes(String needle) => needle.runes.toList();

int? _min(Iterable<int?> values) {
  int? best;
  for (final value in values) {
    if (value != null && (best == null || value < best)) best = value;
  }
  return best;
}

int? _baseMatch(MentionChoice choice, List<int> needle) =>
    _min(choice.aliases.map((name) => foldedNameMatchTier(name, needle)));

/// [mentionMatch] for a normalized, non-empty [needle] and the query's
/// exact public [key], if it is one.
int? _labelMatch(MentionChoice choice, List<int> needle, String? key) {
  // An exact public key is not a name: it matches only its owner.
  if (key != null) return choice.pubkey == key ? 0 : null;
  // No known name: the label is only a key fallback.
  if (choice.aliases.isEmpty) return null;
  final visible = foldedNameMatchTier(choice.label, needle);
  if (visible != null) return visible;
  final base = _baseMatch(choice, needle);
  return base == null ? null : 4 + base;
}

/// A choice's match (section 3): the label tier, else 4 plus the best alias
/// tier. Null means the choice does not match. Keys are never searchable,
/// except an exact public key, which matches only the choice it names.
@visibleForTesting
int? mentionMatch(MentionChoice choice, String query) {
  final needle = normalizeName(query);
  if (needle.isEmpty) return 0;
  return _labelMatch(choice, _runes(needle), typedPublicKey(query));
}

/// Section 4 rule 3: a choice whose shown label matches the query leads when
/// one of its names (aliases, never generated label parts) is the query, or,
/// for the viewer's own agent, continues it with a new word (`Sol 6` for
/// `sol`, not `Solana`). [needle] is normalized and non-empty.
bool _leads(MentionChoice choice, String needle, List<int> needleRunes) {
  if (choice.agent && !choice.owned) return false;
  if (foldedNameMatchTier(choice.label, needleRunes) == null) return false;
  return choice.aliases.any(
    (name) =>
        normalizeName(name) == needle ||
        (choice.agent && nameContinues(name, needle)),
  );
}

int _compare(String a, String b) => a.compareTo(b);

/// Section 4 order. Every rule is a key of one choice, so the result is the
/// same for any input order. Another community's agents follow everyone
/// here; an exact person and the viewer's own agents that start with the
/// query's words lead. Agents that share a name form one block at its
/// lowest label; recency, managed and presence order only that block.
List<MentionChoice> rankMentions(
  Iterable<MentionChoice> choices,
  String query, {
  Map<String, int> history = const {},
  String Function(String pubkey)? presence,
}) {
  final needle = normalizeName(query);
  final needleRunes = _runes(needle);
  final key = typedPublicKey(query);
  final keyed = <_Keyed>[];
  for (final choice in choices) {
    final match = needle.isEmpty ? 0 : _labelMatch(choice, needleRunes, key);
    if (match == null) continue;
    final status = presence?.call(choice.pubkey);
    keyed.add(
      _Keyed(
        choice: choice,
        tier: [
          choice.member ? 0 : 1,
          choice.elsewhere == null ? 0 : 1,
          needle.isNotEmpty && _leads(choice, needle, needleRunes) ? 0 : 1,
          match,
          // No aliases: after every named choice, even for the empty query.
          _baseMatch(choice, needleRunes) ?? 1 << 30,
          choice.agent && choice.owned ? 0 : 1,
        ],
        label: normalizeName(choice.label),
        blockName: choice.agent ? normalizeName(choice.name) : null,
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

bool _hasExactName(MentionChoice choice, String needle) =>
    choice._names.any((name) => normalizeName(name) == needle);

/// Section 5: whether a known longer name keeps the query's exact name from
/// selecting. Once a member has the exact name, only another member's longer
/// name does: a choice outside the channel never makes a member's name
/// ambiguous. A choice's own longer name does not count when that same
/// choice also has the exact name (`Example Agent` beside `Example`), nor
/// does a generated `(agent)` suffix on its label.
bool _continued(Iterable<MentionChoice> choices, String needle) {
  final member = choices.any(
    (c) => c.member && c.aliases.isNotEmpty && _hasExactName(c, needle),
  );
  for (final choice in choices) {
    if (member && !choice.member) continue;
    if (_hasExactName(choice, needle)) continue;
    final longer =
        choice.agent &&
            choice.aliases.any(
              (name) => '${name.trim()} (agent)' == choice.label,
            )
        ? choice.aliases
        : choice._names;
    if (longer.any((name) => nameContinues(name, needle))) return true;
  }
  return false;
}

/// Section 5: the key Space selects, or null. Checked against the full,
/// uncapped choice set. An exact public key names only its owner, even one
/// with no known name, and no name continues it.
String? exactMention(Iterable<MentionChoice> choices, String query) {
  final key = typedPublicKey(query);
  if (key != null) {
    return choices.where((choice) => choice.pubkey == key).firstOrNull?.pubkey;
  }
  final needle = normalizeName(query);
  if (needle.isEmpty || _continued(choices, needle)) return null;
  String? found;
  var count = 0;
  for (final choice in choices) {
    if (choice.aliases.isNotEmpty && _hasExactName(choice, needle)) {
      count++;
      found = choice.pubkey;
    }
  }
  return count == 1 ? found : null;
}

final _mentionTerminators = RegExp(r'''^[ .,!?;:)\]}'’"”]$''', unicode: true);

/// Section 5: whether [character], typed straight after a query, selects
/// its exact, unique name as Space does. Besides Space: closing punctuation,
/// a closing bracket for the ones a query opens after, or a closing quote,
/// so `@Bob,` and `@Bob's` mention Bob. The client keeps the character after
/// the mention in place of the space. Where Space would not select, the
/// character is typed into the query instead: `'` after `Pat O` while
/// `Pat O'Brien` is known stays in the query. Any other character, such as
/// a hyphen, is never a terminator.
bool mentionTerminator(String character) =>
    _mentionTerminators.hasMatch(character);

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
/// is still the start of a known name, compared after section 3 folding of
/// the names. The query itself is not trimmed: a complete name plus a
/// trailing space is prose.
bool matchesMentionQuery(String query, Iterable<String> names) {
  if (!query.contains(' ')) return true;
  final folded = foldName(query);
  final known = names.map(normalizeName).toList();
  if (folded.endsWith(' ') && known.contains(folded.trimRight())) return false;
  return known.any((name) => name.startsWith(folded));
}
