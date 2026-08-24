import 'package:flutter/foundation.dart';

/// The inbox/channel destination currently visible to the user.
@immutable
class VisibleConversation {
  final String channelId;
  final String? messageId;
  final String? threadRootId;

  const VisibleConversation({
    required this.channelId,
    this.messageId,
    this.threadRootId,
  });
}

final Map<Object, VisibleConversation> _visibleConversationOwners = {};

/// The conversation currently on screen, if any.
VisibleConversation? get currentVisibleConversation {
  if (_visibleConversationOwners.isEmpty) return null;
  return _visibleConversationOwners.values.last;
}

/// Register [conversation] for [owner] until the returned callback runs.
void Function() registerVisibleConversation(
  Object owner,
  VisibleConversation conversation,
) {
  _visibleConversationOwners[owner] = conversation;
  return () {
    _visibleConversationOwners.remove(owner);
  };
}

/// Clears all visible-conversation registrations (community remount).
void resetVisibleConversationRegistry() {
  _visibleConversationOwners.clear();
}
