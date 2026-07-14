part of '../channel_detail_page.dart';

class _ReadOnlyNotice extends StatelessWidget {
  final Channel channel;

  const _ReadOnlyNotice({required this.channel});

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: EdgeInsets.only(
        left: Grid.gutter,
        right: Grid.gutter,
        top: Grid.xxs,
        bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.xxs,
      ),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: context.colors.outlineVariant)),
        color: context.colors.surface,
      ),
      child: Text(
        channel.isArchived
            ? 'This ${channel.isForum ? 'forum' : 'channel'} is archived and read-only on mobile.'
            : 'Join this ${channel.isForum ? 'forum' : 'channel'} from Manage to participate.',
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
        textAlign: TextAlign.center,
      ),
    );
  }
}

class _HeaderEphemeralBadge extends StatelessWidget {
  final Channel channel;

  const _HeaderEphemeralBadge({required this.channel});

  @override
  Widget build(BuildContext context) {
    final display = ephemeralChannelDisplay(channel);
    if (display == null) return const SizedBox.shrink();

    return Tooltip(
      message: display.tooltipLabel,
      child: Icon(
        LucideIcons.clockFading,
        key: const Key('chat-ephemeral-badge'),
        size: 16,
        color: context.colors.onSurfaceVariant,
      ),
    );
  }
}

class _DetailConnectionBanner extends StatelessWidget {
  final SessionState state;
  final VoidCallback onRetry;

  const _DetailConnectionBanner({required this.state, required this.onRetry});

  @override
  Widget build(BuildContext context) {
    if (state.status == SessionStatus.connected ||
        state.status == SessionStatus.disconnected) {
      return const SizedBox.shrink();
    }

    if (state.status == SessionStatus.failed) {
      return Material(
        color: context.colors.surfaceContainerHighest,
        child: InkWell(
          onTap: onRetry,
          child: Padding(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.gutter,
              vertical: Grid.quarter + 2,
            ),
            child: Center(
              child: Text(
                state.lastError is RelayAuthRejectedException
                    ? 'Authentication rejected — Retry'
                    : 'Connection lost — Retry',
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ),
        ),
      );
    }

    final message = state.reconnectAttempt > 0
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
