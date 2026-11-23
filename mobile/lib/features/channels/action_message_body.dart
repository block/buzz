import 'package:flutter/material.dart';

import '../../shared/profile/user_profile.dart';
import '../../shared/theme/theme.dart';
import 'small_avatar.dart';
import 'message_reply_drag_target.dart';

/// A narrated action, kept inside the message's normal reply and action surface.
class ActionMessageBody extends StatelessWidget {
  const ActionMessageBody({
    super.key,
    required this.pubkey,
    required this.displayName,
    required this.timestamp,
    required this.onAuthorTap,
    required this.child,
    required this.active,
    this.profile,
    this.isAgent = false,
  });

  final String pubkey;
  final String displayName;
  final String timestamp;
  final UserProfile? profile;
  final bool isAgent;
  final VoidCallback onAuthorTap;
  final Widget child;
  final bool active;

  @override
  Widget build(BuildContext context) {
    if (!active) return child;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          MessageLongPressTarget(
            child: Semantics(
              container: true,
              label: 'Open $displayName profile',
              button: true,
              child: IconButton(
                onPressed: onAuthorTap,
                padding: EdgeInsets.zero,
                constraints: const BoxConstraints(
                  minWidth: Grid.xl,
                  minHeight: Grid.xl,
                ),
                icon: ExcludeSemantics(
                  child: SmallAvatar(
                    pubkey: pubkey,
                    userCache: {pubkey.toLowerCase(): ?profile},
                    agent: isAgent,
                    size: Grid.sm,
                  ),
                ),
              ),
            ),
          ),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                child,
                MessageLongPressTarget(
                  child: Text(
                    timestamp,
                    style: messageTimestampTextStyle.copyWith(
                      color: context.tertiaryTextColor,
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Add a literal profile label to the displayed Markdown, never the saved body.
String actionMessageMarkdown(String displayName, String action) {
  final name = displayName
      .replaceAll(RegExp(r'[\r\n]+'), ' ')
      .replaceAllMapped(RegExp(r'[!-/:-@\[-`{-~]'), (m) => '\\${m[0]}');
  return '**$name** $action';
}
