import 'link_normalizer.dart';

const _shieldBase = 0xE000;

final _escapeOrBreak = RegExp(
  '(?:${proseUrlPattern.pattern})'
  r'|\\(?:[!-/:-@\[-`{-~]|\n(?=[ \t]*\S))',
);
final _shielded = RegExp('[\uE021-\uE07E]+');

/// Applies CommonMark backslash escapes (spec 2.4) outside code spans, fenced
/// code and URLs. Each escaped ASCII punctuation character becomes a private-use
/// stand-in that no Markdown pattern matches; [unshieldBackslashEscapes]
/// restores it for display. A backslash before a line break that continues the
/// paragraph is a hard break and is dropped, since a newline already breaks the
/// line. Any other backslash stays literal.
String shieldBackslashEscapes(String content) => content.contains(r'\')
    ? mapMarkdownProse(
        content,
        (prose) => prose.replaceAllMapped(_escapeOrBreak, (match) {
          final matched = match[0]!;
          if (!matched.startsWith(r'\')) return matched;
          final escaped = matched.codeUnitAt(1);
          return escaped == 0x0A
              ? '\n'
              : String.fromCharCode(_shieldBase + escaped);
        }),
      )
    : content;

/// The characters [shieldBackslashEscapes] held, as authored.
String unshieldBackslashEscapes(String text) => text.replaceAllMapped(
  _shielded,
  (match) => String.fromCharCodes(
    match[0]!.codeUnits.map((unit) => unit - _shieldBase),
  ),
);
