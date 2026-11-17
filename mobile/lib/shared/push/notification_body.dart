import 'package:characters/characters.dart';

/// Longest notification body, in UTF-16 code units. iOS shows at most this
/// much of a body, so every platform's body fits in it. See
/// docs/adr/0002-notification-message-text.md.
const int notificationBodyLimit = 256;

/// Most of a message's content, in UTF-16 code units, that the body is built
/// from. The regexes below can take quadratic time on unclosed markup and
/// content can be up to 256 KB, so a long message is cut first.
const int notificationBodyInputLimit = 4096;

/// Plain-text notification body for a Markdown message, as ADR 0002 defines it
/// and `test-fixtures/notification-body.json` checks it. It is a port of
/// `BuzzPushNotificationResolver.previewBody` in `BuzzPushKit`: change the two
/// together.
///
/// Code and URLs stay as written, line breaks are kept, and the Markdown
/// syntax for code, links, images, bold and strikethrough is removed. Runs of
/// spaces collapse outside code. A body over [notificationBodyLimit] code
/// units, or built from content cut at [notificationBodyInputLimit], keeps
/// whole characters up to one unit short of the limit and ends with "…".
String notificationBody(String content) {
  var input = content;
  final cut = input.length > notificationBodyInputLimit;
  if (cut) {
    final kept = StringBuffer();
    var units = 0;
    for (final character in input.characters) {
      units += character.length;
      if (units > notificationBodyInputLimit) break;
      kept.write(character);
    }
    input = kept.toString();
  }
  input = input
      .replaceAll('\r\n', '\n')
      .replaceAll(_codeOpen, '')
      .replaceAll(_codeClose, '');

  // Swap each code span for a placeholder while the prose rules run over the
  // whole text, so a link label or a bold run can contain code.
  final codes = <String>[];
  final prose = StringBuffer();
  var position = 0;
  for (final match in _code.allMatches(input)) {
    prose.write(input.substring(position, match.start));
    final body = match.group(1) ?? match.group(3) ?? match.group(4)!;
    prose
      ..write(_codeOpen)
      ..write(codes.length)
      ..write(_codeClose);
    codes.add(body);
    position = match.end;
  }
  prose.write(input.substring(position));

  final stripped = _stripProse(prose.toString());
  var result = stripped.replaceAllMapped(
    _codePlaceholder,
    (match) => codes[int.parse(match.group(1)!)],
  );
  result = result
      .replaceAll(RegExp(r'[ \t]+\n'), '\n')
      .replaceAll(RegExp(r'\n{3,}'), '\n\n');
  result = _trim(result);
  if (!cut && result.length <= notificationBodyLimit) return result;

  final kept = StringBuffer();
  var units = 0;
  for (final character in result.characters) {
    units += character.length;
    if (units >= notificationBodyLimit) break;
    kept.write(character);
  }
  return '${_trim(kept.toString())}…';
}

// The patterns below keep ICU regex semantics, which the iOS formatter uses,
// not Dart's. ICU's `\s` is `\p{White_Space}`, listed here, and its `.`
// excludes every line terminator: U+000A to U+000D, U+0085, U+2028, U+2029.
const _space =
    r'\t-\r \u0085\u00A0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000';
const _notLineEnd = r'[^\n-\r\u0085\u2028\u2029]';

/// A fenced span on one line, a fenced code block, or an inline code span. A
/// block's opening line (a fence of three or more backticks, then an optional
/// language) is dropped, and the block closes only at a fence at least as long
/// that ends its line, so backticks inside the code (a "```" string literal)
/// don't end it. A fence with no closing fence runs to the end of the text, as
/// in CommonMark, which also covers a block cut by [notificationBodyInputLimit].
/// Group 2 is the block's fence; the others hold code.
final _code = RegExp(
  r'```([^\n]*?)(?:```|$)'
  r'|(`{3,})[^\n]*\n([\s\S]*?)(?:\n?\2`*[ \t]*(?=\n|$)|$)'
  r'|`([^`\n]*)`',
);

/// Stands in for a code span while the prose rules run. Private-use
/// characters, removed from the content first, so they can't collide.
const _codeOpen = '\uE000';
const _codeClose = '\uE001';
final _codePlaceholder = RegExp('$_codeOpen([0-9]+)$_codeClose');

final _link = RegExp(r'!?\[([^\]]*)\]\([^)]*\)');
final _emphasis = RegExp(
  '(\\*\\*|~~)([^$_space](?:$_notLineEnd*?[^$_space])??)\\1',
);
final _spaces = RegExp(r'[ \t]+');

String _stripProse(String prose) => prose
    .replaceAllMapped(_link, (match) => match.group(1)!)
    .replaceAllMapped(_emphasis, (match) => match.group(2)!)
    .replaceAll(_spaces, ' ');

/// Trims the characters Swift's `whitespacesAndNewlines` does: ICU's
/// whitespace and U+200B. Unlike Dart's `String.trim`, it leaves U+FEFF alone.
final _edgeWhitespace = RegExp('^[$_space\\u200B]+|[$_space\\u200B]+\$');

String _trim(String text) => text.replaceAll(_edgeWhitespace, '');
