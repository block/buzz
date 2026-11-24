/// Desktop-compatible starting title for a plain-text Solo message.
String startingMeChatTitle(String message) {
  var body = message.trim();
  // Source links are context rather than a useful chat title.
  while (body.toLowerCase().startsWith('buzz://')) {
    final end = body.indexOf(RegExp(r'\s'));
    body = end < 0 ? '' : body.substring(end).trimLeft();
  }
  // A quoted source is context too; title from the prompt after it, else
  // from the quote itself.
  final lines = body.split('\n');
  final prompt = lines.skipWhile((line) => line.startsWith('>')).join('\n');
  body = prompt.trim().isNotEmpty
      ? prompt.trim()
      : lines.map((line) => line.replaceFirst(RegExp(r'^>\s?'), '')).join('\n');
  final title = String.fromCharCodes(
    body.replaceAll(RegExp(r'\s+'), ' ').runes.take(80),
  ).trim().replaceFirst(RegExp(r'^[#\s]+'), '').trim();
  return title.isEmpty ? 'Chat' : title;
}
