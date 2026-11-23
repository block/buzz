/// The narrated body of an IRC-style `/me ` message, or null for ordinary text.
///
/// The prefix stays in the signed message. Only a leading, lowercase command
/// followed by a space and a nonempty action changes its presentation.
String? messageAction(String content) {
  if (!content.startsWith('/me ')) return null;
  final action = content.substring(4).trim();
  return action.isEmpty ? null : action;
}
