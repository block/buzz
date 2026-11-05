/// A draft that runs [command] with [args], the text after the command token.
typedef SlashInvocation<T> = ({T command, String name, String args});

/// An open `/` autocomplete: `[start, end)` covers the whole first token and
/// [query] is the text after the `/`.
typedef SlashQuery = ({int start, int end, String query});

final _whitespace = RegExp(r'\s');
final _nameCharacters = RegExp(r'^[A-Za-z0-9_-]*$');

/// Spec "Recognition": the draft is a command when, after leading whitespace,
/// its first token is `/` plus a name [lookup] knows, ending at whitespace or
/// the end of the draft. Matching is case-insensitive. Anything else,
/// including a draft that starts with a mention, is an ordinary message.
SlashInvocation<T>? recognizeSlashDraft<T>(
  String draft,
  T? Function(String name) lookup,
) {
  final text = draft.trimLeft();
  if (!text.startsWith('/')) return null;
  final match = _whitespace.firstMatch(text);
  final end = match?.start ?? text.length;
  final name = text.substring(1, end).toLowerCase();
  if (name.isEmpty) return null;
  final command = lookup(name);
  if (command == null) return null;
  return (command: command, name: name, args: text.substring(end).trim());
}

/// Spec "Autocomplete": the open query when only whitespace precedes the
/// first token, that token is `/` plus name characters, and [caret] is in it.
SlashQuery? findSlashQuery(String text, int caret) {
  var start = 0;
  while (start < text.length && _whitespace.hasMatch(text[start])) {
    start++;
  }
  if (start >= text.length || text[start] != '/') return null;
  final match = _whitespace.firstMatch(text.substring(start));
  final end = match == null ? text.length : start + match.start;
  if (caret <= start || caret > end) return null;
  final query = text.substring(start + 1, end);
  if (!_nameCharacters.hasMatch(query)) return null;
  return (start: start, end: end, query: query);
}
