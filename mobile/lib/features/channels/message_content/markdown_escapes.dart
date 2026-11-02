import 'link_normalizer.dart';

/// Holds each backslash-escaped character as a private-use stand-in, so no
/// link, emphasis, mention, emoji, channel or math syntax matches it: `\*x\*`
/// stays `*x*` and `\@alice` stays text.
String shieldMarkdownEscapes(String text) => text.contains(r'\')
    ? text.replaceAllMapped(
        _escape,
        (m) => String.fromCharCode(_escapeBase + m[1]!.codeUnitAt(0)),
      )
    : text;

/// [shieldMarkdownEscapes] applied outside inline and fenced code, where a
/// backslash is literal.
String shieldMarkdownProseEscapes(String content) => content.contains(r'\')
    ? mapMarkdownProse(content, shieldMarkdownEscapes)
    : content;

/// The characters [shieldMarkdownEscapes] held, as displayed.
String unshieldMarkdownEscapes(String text) => text.replaceAllMapped(
  shieldedMarkdownEscape,
  (m) => String.fromCharCode(m[0]!.codeUnitAt(0) - _escapeBase),
);

/// The escapes [shieldMarkdownEscapes] held, back in source form, for helpers
/// that read Markdown.
String reescapeMarkdownEscapes(String text) => text.replaceAllMapped(
  shieldedMarkdownEscape,
  (m) => '\\${String.fromCharCode(m[0]!.codeUnitAt(0) - _escapeBase)}',
);

/// Matches one character held by [shieldMarkdownEscapes].
final shieldedMarkdownEscape = RegExp('[\uE021-\uE07E]');

const _escapeBase = 0xE000;
// CommonMark: a backslash escapes any ASCII punctuation character.
final _escape = RegExp(r'\\([!-/:-@\[-`{-~])');
