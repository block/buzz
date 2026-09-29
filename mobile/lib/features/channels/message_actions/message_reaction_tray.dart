part of '../message_actions.dart';

class _MessageReactionTray extends StatelessWidget {
  final Animation<double> animation;
  final double trayWidth;
  final TimelineMessage message;
  final BuildContext pageContext;
  final WidgetRef pageRef;
  final Object popResult;
  final void Function(Object? result, VoidCallback effect) onSelected;

  const _MessageReactionTray({
    required this.animation,
    required this.trayWidth,
    required this.message,
    required this.pageContext,
    required this.pageRef,
    required this.popResult,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    return _AnimatedReactionTray(
      trayKey: const ValueKey('message-action-reaction-tray'),
      animation: animation,
      trayWidth: trayWidth,
      scaleAlignment: Alignment.bottomLeft,
      message: message,
      pageContext: pageContext,
      pageRef: pageRef,
      popResult: popResult,
      onSelected: onSelected,
    );
  }
}
