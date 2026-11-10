part of '../channels_page.dart';

class _EphemeralBadge extends StatelessWidget {
  final Channel channel;

  const _EphemeralBadge({required this.channel});

  @override
  Widget build(BuildContext context) {
    final display = ephemeralChannelDisplay(channel);
    if (display == null) return const SizedBox.shrink();

    return Tooltip(
      message: display.tooltipLabel,
      child: SizedBox(
        width: _kChannelIconSize,
        child: Center(
          child: Icon(
            BuzzIcons.clockFading,
            key: Key('channel-ephemeral-${channel.id}'),
            size: 16,
            color: context.colors.onSurfaceVariant,
          ),
        ),
      ),
    );
  }
}

class _ErrorView extends HookConsumerWidget {
  final Object error;
  final VoidCallback onRetry;

  const _ErrorView({required this.error, required this.onRetry});

  static String _userMessage(Object error) {
    if (error is RelayException) {
      // There is no API token to check. A protected community's Retry gets a
      // fresh token; once the organization sign-in has ended, the shell
      // banner owns the action instead (see build).
      if (error.statusCode == 401) {
        return "This community couldn't authenticate the request. Try again.";
      }
      if (error.statusCode == 403) {
        return 'Access denied.';
      }
      return 'Server error (${error.statusCode}). Try again later.';
    }
    if (error is SocketException) {
      return 'Could not reach the relay server.';
    }
    return 'Something went wrong. Check your connection.';
  }

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final signIn = ref.watch(organizationSignInProvider);
    final auth = ref.watch(hostedRelayAuthProvider);
    useListenable(signIn);
    useListenable(auth);
    // With the organization sign-in ended, a read goes out unsigned and Retry
    // cannot succeed. The Home banner then shows the working action.
    final signInOwned = organizationSignInPromptVisible(
      signIn,
      auth,
      ref.watch(relayConfigProvider),
    );
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(BuzzIcons.wifiOff, size: Grid.xl, color: context.colors.error),
            const SizedBox(height: Grid.xs),
            Text(
              'Could not load channels',
              style: context.textTheme.titleMedium,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              signInOwned
                  ? 'Use the sign-in prompt below to reconnect.'
                  : _userMessage(error),
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
            ),
            if (!signInOwned) ...[
              const SizedBox(height: Grid.xs),
              FilledButton.icon(
                onPressed: onRetry,
                icon: const Icon(BuzzIcons.refreshCw),
                label: const Text('Retry'),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
