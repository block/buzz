/// Whether a message is a quiet agent message: its signed `audience` tag is
/// `agents`. Agents send progress and agent-to-agent detail this way, and the
/// completion message to everyone. As on desktop (`buzz-app`'s
/// `docs/unread.md`), a quiet message never counts as unread or earns
/// attention, even when it mentions the reader.
bool isQuietAgentMessage(List<List<String>> tags) => tags.any(
  (tag) => tag.length >= 2 && tag[0] == 'audience' && tag[1] == 'agents',
);
