import 'package:flutter/foundation.dart';

import 'nostr_models.dart';

/// A user's NIP-38 status (kind:30315, d=general).
@immutable
class UserStatus {
  final String text;
  final String emoji;
  final int updatedAt;

  const UserStatus({
    required this.text,
    required this.emoji,
    required this.updatedAt,
  });

  factory UserStatus.fromEvent(NostrEvent event) {
    final emojiTag = event.tags
        .where((t) => t.length >= 2 && t[0] == 'emoji')
        .firstOrNull;
    return UserStatus(
      text: event.content,
      emoji: emojiTag?[1] ?? '',
      updatedAt: event.createdAt,
    );
  }

  bool get isEmpty => text.isEmpty && emoji.isEmpty;
}
