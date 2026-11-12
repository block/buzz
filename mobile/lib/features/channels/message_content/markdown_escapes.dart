import 'dart:convert';

import 'package:flutter/painting.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import 'link_normalizer.dart';

/// Holds each backslash-escaped character as a stand-in, so no link,
/// emphasis, mention, emoji, channel or math syntax matches it: `\*x\*` stays
/// `*x*` and `\@alice` stays text.
///
/// A stand-in is an invisible [_marker] followed by the character's fullwidth
/// form, so text the renderer shows without restoring it still reads right.
/// An authored [_marker] becomes two markers, so authored text comes back from
/// [unshieldMarkdownEscapes] unchanged.
String shieldMarkdownEscapes(String text) => _needsShield(text)
    ? text.replaceAllMapped(
        _escape,
        (m) => m[1] == null
            ? '$_marker$_marker'
            : '$_marker${String.fromCharCode(_fullwidthOffset + m[1]!.codeUnitAt(0))}',
      )
    : text;

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
final _escape = RegExp(r'\\([!-/:-@\[-`{-~])|\u2060');

/// [content] with `\[`, `\]`, `\(` and `\)` outside code written as numeric
/// entities, and `\@`, `\#` and `\:` as stand-ins [escapedSigilPattern]
/// draws.
///
/// gpt_markdown reads the bracket and parenthesis pairs as LaTeX delimiters,
/// and runs its inline patterns (mentions, channels, custom emoji) on the
/// source and again on the parsed text, after escapes and entities decode. In
/// CommonMark, and on desktop, all of them are escapes for the literal
/// character. So `\[not a link\](x)` shows `[not a link](x)`, not a formula,
/// and `\@alice` shows `@alice`, not a mention. `\\` stays an escaped
/// backslash.
///
/// A stand-in is [_standInMark] followed by a second character. An authored
/// [_standInMark] is written as two, so authored text is never drawn as a
/// sigil.
String escapeMarkdownLiterals(String content) {
  if (!_literalEscape.hasMatch(content) && !content.contains(_standInMark)) {
    return content;
  }
  return mapMarkdownProse(
    content,
    (prose) => prose.replaceAllMapped(
      _backslashPair,
      (m) => switch (m[1]) {
        null => '$_standInMark$_standInMark',
        '[' => '&#91;',
        ']' => '&#93;',
        '(' => '&#40;',
        ')' => '&#41;',
        final sigil when _sigils.contains(sigil) =>
          _standInMark +
              String.fromCharCode(_standInBase + 1 + _sigils.indexOf(sigil)),
        _standInMark => '\\$_standInMark$_standInMark',
        _ => m[0]!,
      },
    ),
  );
}

/// Draws the stand-ins [escapeMarkdownLiterals] writes as the characters they
/// hold, in every scope, so no pattern claims them.
final escapedSigilPattern = InlinePattern(
  pattern: _standIn,
  scopes: MarkdownComponent.allScopes,
  builder: (context, match, style) =>
      TextSpan(text: _heldBy(match[0]!), style: style),
);

String _heldBy(String standIn) {
  final held = standIn.codeUnitAt(1) - _standInBase;
  return held == 0 ? _standInMark : _sigils[held - 1];
}

const _sigils = '@#:';
// Private-use characters: the mark, then one per sigil.
const _standInBase = 0xE0A0;
const _standInMark = '\u{E0A0}';
final _standIn = RegExp('\u{E0A0}[\u{E0A0}-\u{E0A3}]');
final _literalEscape = RegExp(r'\\[\[\]()@#:]');
final _backslashPair = RegExp('\\\\(.)|\u{E0A0}', dotAll: true);

/// [text] with gpt_markdown's inline-pattern placeholders put back as the
/// source they replaced.
///
/// gpt_markdown 1.3.4 lifts each inline pattern match (mention, channel,
/// emoji) out of the source before parsing, as U+E012, the pattern index, a
/// colon, the Base64 of the UTF-8 match, then U+E013, and puts it back only in
/// text spans. It also masks inside link destinations, whose URL it hands over
/// still masked, so a destination containing `:tada:` would open the
/// placeholder instead of the authored URL.
///
/// Sigil stand-ins from [escapeMarkdownLiterals] go back to their characters
/// too: an escape in a destination is the literal character.
String unmaskInlinePatterns(String text) {
  final unmasked = text.contains('\u{E012}')
      ? text.replaceAllMapped(_patternMask, (m) {
          try {
            return utf8.decode(base64Decode(m[1]!));
          } on FormatException {
            return m[0]!;
          }
        })
      : text;
  return unmasked.replaceAllMapped(_standIn, (m) => _heldBy(m[0]!));
}

final _patternMask = RegExp('\u{E012}\\d+:([A-Za-z0-9+/=]*)\u{E013}');
