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
/// URLs get no stand-ins: gpt_markdown replaces inline-pattern matches with
/// placeholders even inside a URL, so a stand-in there could never be
/// restored. A backslash inside an angle autolink such as
/// `<https://e.com/a\>` is literal (CommonMark), so it stays. In a link
/// destination or a bare URL each escape is resolved to its character
/// instead, so `[x](https://e.com/a\_b)` links to `https://e.com/a_b`.
String shieldMarkdownEscapes(String text) => _needsShield(text)
    ? text.replaceAllMapped(_escape, (m) {
        if (m[2] != null) return m[2]!.replaceAll(_marker, '$_marker$_marker');
        if (m[3] != null) return _resolveUrlEscapes(m[3]!);
        return m[1] == null
            ? '$_marker$_marker'
            : '$_marker${String.fromCharCode(_fullwidthOffset + m[1]!.codeUnitAt(0))}';
      })
    : text;

/// [url] with each escape replaced by its character. An escaped backslash
/// becomes `%5C`, so the renderer cannot read it as a new escape.
String _resolveUrlEscapes(String url) => url.replaceAllMapped(
  _urlEscape,
  (m) => switch (m[1]) {
    null => '$_marker$_marker',
    r'\' => '%5C',
    final held => held,
  },
);

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
// An escaped `\<` or `\]` matches first, so it never opens an autolink or
// a destination. A bare URL takes an escape only before a letter or digit,
// so trailing escapes such as `a\**` stay outside it, as the renderer reads
// them.
final _escape = RegExp(
  r'\\([!-/:-@\[-`{-~])'
  r'|(<[A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*>)'
  r'|(\]\([^\s()]*\)'
  r'|(?:https?://|buzz://|www\.)'
  r'(?:[^\s<>\\]|\\[!-/:-@\[-`{-~](?=[A-Za-z0-9]))*)'
  r'|\u2060',
);
final _urlEscape = RegExp(r'\\([!-/:-@\[-`{-~])|\u2060');
