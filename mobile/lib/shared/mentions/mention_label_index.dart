/// A character a case mapping changes. Any other character matches only
/// itself when case is ignored.
final _cased = RegExp(r'\p{CWCM}', unicode: true);

/// Two characters the regular expression engine reads as one when case is
/// ignored: a back-reference compares exactly as a literal does.
final _sameIgnoringCase = RegExp(
  r'^(.)\1$',
  caseSensitive: false,
  unicode: true,
  dotAll: true,
);

/// One representative per group of non-ASCII characters equal ignoring
/// case, in the order they were first seen, after the ASCII letters.
final _representatives = <int>[for (var c = 0x61; c <= 0x7a; c++) c];
final _representativeOf = <int, int>{};

int _representative(int rune) {
  if (rune < 0x80) return rune >= 0x41 && rune <= 0x5a ? rune + 0x20 : rune;
  return _representativeOf[rune] ??= () {
    if (!_cased.hasMatch(String.fromCharCode(rune))) return rune;
    for (final known in _representatives) {
      if (_sameIgnoringCase.hasMatch(String.fromCharCodes([known, rune]))) {
        return known;
      }
    }
    _representatives.add(rune);
    return rune;
  }();
}

/// [text] with every character replaced by its case group's representative.
/// Two strings fold alike exactly when a case-insensitive Unicode pattern
/// built from one matches the other, which is how a mention label has
/// always been compared with message text: `ſ` and the Kelvin sign fold
/// with `s` and `k`, a final `ς` with `σ`, while `ı` and `İ` stay apart
/// from `i`. `String.toLowerCase` disagrees on those and knows fewer
/// scripts, so the groups are asked of the engine, once per character for
/// the life of the process; there are about 1,500 groups in all. Folding
/// never changes a string's length in UTF-16 units.
String foldMentionLabel(String text) {
  for (var i = 0; i < text.length; i++) {
    final unit = text.codeUnitAt(i);
    if (unit >= 0x80 || (unit >= 0x41 && unit <= 0x5a)) {
      return String.fromCharCodes(text.runes.map(_representative));
    }
  }
  return text;
}

/// Mention labels indexed for `mentionOccurrences`: each label under its
/// folded form, and the distinct label lengths, longest first. Looking a
/// name up costs one hash probe per length, however many labels there are,
/// so build this once for a channel roster and reuse it for every scan.
class MentionLabelIndex {
  MentionLabelIndex(Iterable<String> labels) {
    final lengths = <int>{};
    for (final label in labels) {
      if (label.isEmpty) continue;
      // The first label wins among labels equal ignoring case.
      _byFolded.putIfAbsent(foldMentionLabel(label), () => label);
      lengths.add(label.length);
    }
    this.lengths = lengths.toList()..sort((a, b) => b.compareTo(a));
  }

  static final empty = MentionLabelIndex(const []);

  final _byFolded = <String, String>{};

  /// The distinct lengths of the indexed labels in UTF-16 units, longest
  /// first.
  late final List<int> lengths;

  /// The indexed label that [folded], a [foldMentionLabel] result, spells.
  String? operator [](String folded) => _byFolded[folded];
}
