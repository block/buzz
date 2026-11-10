part of 'buzz_toast.dart';

/// Parent-owned persistent feedback with the same motion as transient toasts.
class ReconnectCapsule extends HookWidget {
  const ReconnectCapsule({
    super.key,
    required this.onSignIn,
    this.visible = true,
    this.message = 'Reconnect to Builderlab',
    this.actionLabel = 'Sign in',
    this.destructiveAction = false,
    this.secondaryAction,
    this.islandOrigin,
    this.islandSpeed = 0.75,
    this.inline = false,
  });

  final VoidCallback onSignIn;
  final bool visible;

  /// Persistent status displayed beside or above the actions.
  final String message;

  /// Label for the primary recovery action.
  final String actionLabel;

  /// Uses the theme error colors for the primary action.
  final bool destructiveAction;

  /// Adds a second recovery action and stacks both buttons below the message.
  final ({String label, VoidCallback onPressed})? secondaryAction;
  final Rect? islandOrigin;
  final double islandSpeed;

  /// Reserves space in a page layout instead of overlaying its content.
  final bool inline;

  @override
  Widget build(BuildContext context) {
    final transientVisible = useValueListenable(_toastCoordinator(context));
    // Remove immediately so neither text nor island surfaces overlap.
    // Remounting after dismissal restarts the persistent entrance if needed.
    if (transientVisible) return const SizedBox.shrink();
    return _ToastBubble(
      message: message,
      actionLabel: actionLabel,
      destructiveAction: destructiveAction,
      secondaryAction: secondaryAction,
      onRetry: onSignIn,
      onDismiss: () {},
      persistent: true,
      visible: visible,
      islandOrigin: islandOrigin,
      islandSpeed: islandSpeed,
      inline: inline,
    );
  }
}
