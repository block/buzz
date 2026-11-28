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
    this.showTimestamp = true,
  });

  final String pubkey;
  final String displayName;
  final String timestamp;
  final UserProfile? profile;
  final bool isAgent;
  final VoidCallback onAuthorTap;
  final Widget child;
  final bool active;

  /// Consecutive actions in one minute share the first action's visible clock.
  final bool showTimestamp;

  /// Leading width shared by narration and its reply/reaction controls.
  static double contentInset(BuildContext context) =>
      messageAvatarSize + messageAvatarContentGap;

  @override
  Widget build(BuildContext context) {
    if (!active) return child;
    final scaler = MediaQuery.textScalerOf(context);
    final firstLineHeight =
        scaler.scale(messageBodyTextStyle.fontSize!) *
        messageBodyTextStyle.height!;
    final textInset = ((Grid.xl - firstLineHeight) / 2).clamp(0.0, Grid.xl);
    final avatarInset = ((firstLineHeight - Grid.xl) / 2).clamp(
      0.0,
      double.infinity,
    );
    return Padding(
      padding: EdgeInsets.only(
        top: showTimestamp ? Grid.half : Grid.quarter,
        bottom: Grid.half,
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (showTimestamp)
            Padding(
              padding: EdgeInsetsDirectional.only(
                start: contentInset(context),
                bottom: Grid.half,
              ),
              child: MessageLongPressTarget(
                child: Text(
                  timestamp,
                  style: messageTimestampTextStyle.copyWith(
                    color: context.tertiaryTextColor,
                  ),
                ),
              ),
            ),
          if (!showTimestamp) Semantics(label: timestamp),
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(
                padding: EdgeInsets.only(top: avatarInset),
                child: MessageLongPressTarget(
                  child: Semantics(
                    container: true,
                    label: 'Open $displayName profile',
                    button: true,
                    child: IconButton(
                      onPressed: onAuthorTap,
                      alignment: AlignmentDirectional.centerStart,
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
              ),
              Expanded(
                child: Padding(
                  padding: EdgeInsets.only(top: textInset),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      // Keep the gesture target on the narration's painted width.
                      DefaultTextStyle.merge(
                        textWidthBasis: TextWidthBasis.longestLine,
                        child: MessageReplyDragTarget(child: child),
                      ),
                    ],
                  ),
                ),
              ),
            ],
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
  return '$name $action';
}
