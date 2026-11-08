import 'package:flutter/widgets.dart';

import '../theme/app_theme.dart';

/// Shared conversation corners for media, voice notes, and huddle cards.
const conversationAttachmentCorners = BorderRadius.all(
  Radius.circular(Radii.container),
);

/// Shares conversation corner geometry with attachment surfaces.
class MessageAttachmentCorners extends InheritedWidget {
  final BorderRadius borderRadius;

  const MessageAttachmentCorners({
    super.key,
    required this.borderRadius,
    required super.child,
  });

  /// Returns the conversation corners, or null outside a message bubble.
  static BorderRadius? of(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<MessageAttachmentCorners>()
      ?.borderRadius;

  @override
  bool updateShouldNotify(MessageAttachmentCorners oldWidget) =>
      borderRadius != oldWidget.borderRadius;
}
