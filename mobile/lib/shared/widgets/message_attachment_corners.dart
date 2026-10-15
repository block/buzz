import 'package:flutter/widgets.dart';

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
