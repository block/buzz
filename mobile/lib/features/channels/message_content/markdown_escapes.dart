import 'link_normalizer.dart';

/// Holds each backslash-escaped character as a private-use stand-in, so no
/// link, emphasis, mention, emoji, channel or math syntax matches it: `\*x\*`
/// stays `*x*` and `\@alice` stays text.
///
/// A stand-in is [_marker] followed by the character shifted into the
/// private-use area. An authored [_marker] becomes two markers, so authored
/// private-use characters come back from [unshieldMarkdownEscapes] unchanged.
String shieldMarkdownEscapes(String text) => _needsShield(text)
    ? text.replaceAllMapped(
        _escape,
        (m) => m[1] == null
            ? '$_marker$_marker'
            : '$_marker${String.fromCharCode(_escapeBase + m[1]!.codeUnitAt(0))}',
      )
    : text;

/// [shieldMarkdownEscapes] applied outside inline and fenced code, where a
/// backslash is literal.
String shieldMarkdownProseEscapes(String content) => _needsShield(content)
    ? mapMarkdownProse(content, shieldMarkdownEscapes)
    : content;

/// The characters [shieldMarkdownEscapes] held, as displayed.
String unshieldMarkdownEscapes(String text) =>
    text.replaceAllMapped(shieldedMarkdownEscape, (m) => _held(m[0]!));

/// The escapes [shieldMarkdownEscapes] held, back in source form, for helpers
/// that read Markdown.
String reescapeMarkdownEscapes(String text) => text.replaceAllMapped(
  shieldedMarkdownEscape,
  (m) => m[0] == '$_marker$_marker' ? _marker : '\\${_held(m[0]!)}',
);

/// Matches one stand-in created by [shieldMarkdownEscapes].
final shieldedMarkdownEscape = RegExp('\uE000[\uE000\uE021-\uE07E]');

bool _needsShield(String text) => text.contains(r'\') || text.contains(_marker);

String _held(String standIn) {
  final unit = standIn.codeUnitAt(1);
  return unit == _escapeBase
      ? _marker
      : String.fromCharCode(unit - _escapeBase);
}

const _marker = '\uE000';
const _escapeBase = 0xE000;
// CommonMark: a backslash escapes any ASCII punctuation character.
final _escape = RegExp(r'\\([!-/:-@\[-`{-~])|\uE000');
