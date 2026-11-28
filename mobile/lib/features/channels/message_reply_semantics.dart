import 'package:flutter/widgets.dart';

/// Offers a message's reply to assistive technologies.
///
/// The swipe that replies is excluded from semantics, and a [Semantics]
/// around it would merge into the row's parent node. The message's
/// long-press region reads this scope so the action sits on the row's own
/// node, beside its long press.
class MessageReplySemantics extends InheritedWidget {
  final VoidCallback onReply;

  const MessageReplySemantics({
    super.key,
    required this.onReply,
    required super.child,
  });

  static VoidCallback? maybeOf(BuildContext context) => context
      .dependOnInheritedWidgetOfExactType<MessageReplySemantics>()
      ?.onReply;

  @override
  bool updateShouldNotify(MessageReplySemantics oldWidget) =>
      onReply != oldWidget.onReply;
}
