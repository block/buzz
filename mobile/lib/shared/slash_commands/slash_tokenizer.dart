import 'package:flutter/foundation.dart';

/// A whitespace-delimited argument and where it sits in the argument string.
@immutable
class SlashToken {
  final String text;
  final int start;
  final int end;

  const SlashToken(this.text, this.start, this.end);

  @override
  bool operator ==(Object other) =>
      other is SlashToken &&
      other.text == text &&
      other.start == start &&
      other.end == end;

  @override
  int get hashCode => Object.hash(text, start, end);

  @override
  String toString() => 'SlashToken($text, $start, $end)';
}

final _token = RegExp(r'\S+');

/// Splits [args] on whitespace. Quotes have no meaning: names with spaces
/// arrive as resolved mentions, not as text.
List<SlashToken> tokenizeSlashArgs(String args) => [
  for (final match in _token.allMatches(args))
    SlashToken(match.group(0)!, match.start, match.end),
];

/// The part of [args] from token [index] on, with its own spacing intact.
/// Empty when there are fewer tokens.
String slashArgsFrom(String args, int index) {
  final tokens = tokenizeSlashArgs(args);
  if (index >= tokens.length) return '';
  return args.substring(tokens[index].start).trimRight();
}
