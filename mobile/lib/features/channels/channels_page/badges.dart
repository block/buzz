part of '../channels_page.dart';

class _UnreadBadge extends StatelessWidget {
  final String channelId;
  final int count;

  const _UnreadBadge({required this.channelId, required this.count});

  @override
  Widget build(BuildContext context) {
    if (count <= 0) {
      return SizedBox(
        key: Key('channel-unread-dot-$channelId'),
        width: 20,
        height: 20,
        child: Center(
          child: Container(
            width: 8,
            height: 8,
            decoration: BoxDecoration(
              color: context.colors.primary,
              shape: BoxShape.circle,
            ),
            child: Semantics(label: 'unread'),
          ),
        ),
      );
    }

    return Container(
      key: Key('channel-unread-$channelId'),
      constraints: const BoxConstraints(minWidth: 20, minHeight: 20),
      padding: const EdgeInsets.symmetric(horizontal: Grid.quarter),
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: context.colors.primary,
        borderRadius: BorderRadius.circular(999),
      ),
      child: Text(
        _formatUnreadCount(count),
        style: context.textTheme.labelSmall?.copyWith(
          color: context.colors.onPrimary,
          fontSize: 10,
          fontWeight: FontWeight.w700,
          height: 1,
        ),
      ),
    );
  }
}

String _formatUnreadCount(int count) => count > 99 ? '99+' : count.toString();

class _EphemeralBadge extends StatelessWidget {
  final Channel channel;

  const _EphemeralBadge({required this.channel});

  @override
  Widget build(BuildContext context) {
    final display = ephemeralChannelDisplay(channel);
    if (display == null) return const SizedBox.shrink();

    return Tooltip(
      message: display.tooltipLabel,
      child: Icon(
        LucideIcons.clockFading,
        key: Key('channel-ephemeral-${channel.id}'),
        size: 16,
        color: context.colors.onSurfaceVariant,
      ),
    );
  }
}

class _ConnectionBanner extends StatelessWidget {
  final SessionState state;

  const _ConnectionBanner({required this.state});

  @override
  Widget build(BuildContext context) {
    if (state.status == SessionStatus.connected ||
        state.status == SessionStatus.disconnected ||
        state.status == SessionStatus.failed) {
      return const SizedBox.shrink();
    }

    final isConnecting = state.status == SessionStatus.connecting;
    final message = isConnecting
        ? 'Connecting…'
        : state.reconnectAttempt > 0
        ? 'Reconnecting… (attempt ${state.reconnectAttempt})'
        : 'Reconnecting…';

    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.quarter + 2,
      ),
      color: context.colors.surfaceContainerHighest,
      child: Row(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          SizedBox(
            width: 12,
            height: 12,
            child: CircularProgressIndicator(
              strokeWidth: 2,
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(width: Grid.xxs),
          Text(
            message,
            style: context.textTheme.labelSmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}

class _ConnectionLostBanner extends StatelessWidget {
  final Object? error;
  final VoidCallback onRetry;

  const _ConnectionLostBanner({required this.error, required this.onRetry});

  @override
  Widget build(BuildContext context) {
    final message = error is RelayAuthRejectedException
        ? 'Authentication rejected — Retry'
        : 'Connection lost — Retry';
    return Material(
      color: context.colors.surfaceContainerHighest,
      child: InkWell(
        onTap: onRetry,
        child: SizedBox(
          height: _kBannerHeight,
          child: Center(
            child: Text(
              message,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _OfflineView extends StatelessWidget {
  final Object? error;
  final String relayHost;
  final VoidCallback onRetry;

  const _OfflineView({
    required this.error,
    required this.relayHost,
    required this.onRetry,
  });

  @override
  Widget build(BuildContext context) {
    final authRejected = error is RelayAuthRejectedException;
    final headline = authRejected
        ? 'Authentication rejected by the relay'
        : "Can't reach your workspace";
    final body = authRejected
        ? 'Check your workspace access or credentials, then retry.'
        : 'Check your network or VPN (e.g. Cloudflare WARP), then retry.';
    final detail = relayHost;

    return Center(
      child: Padding(
        padding: const EdgeInsets.all(Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.wifiOff,
              size: Grid.xl,
              color: context.colors.error,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              headline,
              style: context.textTheme.titleMedium,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              body,
              style: context.textTheme.bodyMedium,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              detail,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: Grid.xs),
            FilledButton.icon(
              onPressed: onRetry,
              icon: const Icon(LucideIcons.refreshCw),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }
}

class _ErrorView extends StatelessWidget {
  final Object error;
  final VoidCallback onRetry;

  const _ErrorView({required this.error, required this.onRetry});

  static String _userMessage(Object error) {
    if (error is RelayException) {
      if (error.statusCode == 401) {
        return 'Not authorized. Check your API token.';
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
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(Grid.sm),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              LucideIcons.wifiOff,
              size: Grid.xl,
              color: context.colors.error,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'Could not load channels',
              style: context.textTheme.titleMedium,
            ),
            const SizedBox(height: Grid.xxs),
            Text(
              _userMessage(error),
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
              textAlign: TextAlign.center,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
            ),
            const SizedBox(height: Grid.xs),
            FilledButton.icon(
              onPressed: onRetry,
              icon: const Icon(LucideIcons.refreshCw),
              label: const Text('Retry'),
            ),
          ],
        ),
      ),
    );
  }
}
