import 'link_normalizer.dart';

/// Holds each backslash-escaped character as a stand-in, so no link,
/// emphasis, mention, emoji, channel or math syntax matches it: `\*x\*` stays
/// `*x*` and `\@alice` stays text.
///
/// A stand-in is an invisible [_marker] followed by the character's fullwidth
/// form, so text the renderer shows without restoring it still reads right.
/// An authored [_marker] becomes two markers, so authored text comes back from
/// [unshieldMarkdownEscapes] unchanged.
///
/// A backslash inside an angle autolink such as `<https://e.com/a\>` is
/// literal (CommonMark), so autolinks keep theirs and only double markers.
String shieldMarkdownEscapes(String text) => _needsShield(text)
    ? text.replaceAllMapped(_escape, (m) {
        if (m[2] != null) return m[2]!.replaceAll(_marker, '$_marker$_marker');
        return m[1] == null
            ? '$_marker$_marker'
            : '$_marker${String.fromCharCode(_fullwidthOffset + m[1]!.codeUnitAt(0))}';
      })
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
final shieldedMarkdownEscape = RegExp('\u2060[\u2060\uFF01-\uFF5E]');

bool _needsShield(String text) => text.contains(r'\') || text.contains(_marker);

String _held(String standIn) {
  final held = standIn.substring(1);
  return held == _marker
      ? _marker
      : String.fromCharCode(held.codeUnitAt(0) - _fullwidthOffset);
}

// WORD JOINER: zero-width, not whitespace, and not Markdown syntax.
const _marker = '\u2060';
// ASCII 0x21-0x7E to FULLWIDTH 0xFF01-0xFF5E.
const _fullwidthOffset = 0xFEE0;
// CommonMark: a backslash escapes any ASCII punctuation character.
// An escaped `\<` matches first, so it never opens an autolink.
final _escape = RegExp(
  r'\\([!-/:-@\[-`{-~])|(<[A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*>)|\u2060',
);
