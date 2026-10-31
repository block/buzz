part of 'buzz_toast.dart';

/// Parent-owned persistent feedback with the same motion as transient toasts.
class ReconnectCapsule extends StatelessWidget {
  const ReconnectCapsule({
    super.key,
    required this.onSignIn,
    this.visible = true,
    this.islandOrigin,
    this.islandSpeed = 0.75,
  });

  final VoidCallback onSignIn;
  final bool visible;
  final Rect? islandOrigin;
  final double islandSpeed;

  @override
  Widget build(BuildContext context) => _ToastBubble(
    message: 'Sign in to Builderbot to reconnect',
    actionLabel: 'Sign in',
    onRetry: onSignIn,
    onDismiss: () {},
    persistent: true,
    visible: visible,
    islandOrigin: islandOrigin,
    islandSpeed: islandSpeed,
  );
}
