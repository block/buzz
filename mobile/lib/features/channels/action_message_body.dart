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

  /// Leading width shared by narration and its reply/reaction controls.
  static double contentInset(BuildContext context) =>
      _timeGutterWidth(context) + Grid.xl;

  @override
  Widget build(BuildContext context) {
    if (!active) return child;
    final scaler = MediaQuery.textScalerOf(context);
    double topInset(TextStyle style) =>
        ((Grid.xl - scaler.scale(style.fontSize!) * style.height!) / 2).clamp(
          0.0,
          Grid.xl,
        );
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: Grid.half),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          SizedBox(
            width: _timeGutterWidth(context),
            child: Padding(
              padding: EdgeInsetsDirectional.only(
                end: Grid.half,
                top: topInset(messageTimestampTextStyle),
              ),
              child: Align(
                alignment: AlignmentDirectional.centerEnd,
                child: MessageLongPressTarget(
                  child: Text(
                    timestamp,
                    maxLines: 1,
                    style: messageTimestampTextStyle.copyWith(
                      color: context.tertiaryTextColor,
                    ),
                  ),
                ),
              ),
            ),
          ),
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
            child: Padding(
              padding: EdgeInsets.only(top: topInset(messageBodyTextStyle)),
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

double _timeGutterWidth(BuildContext context) =>
    Grid.xxl *
    MediaQuery.textScalerOf(
      context,
    ).scale(messageTimestampTextStyle.fontSize!) /
    messageTimestampTextStyle.fontSize!;
