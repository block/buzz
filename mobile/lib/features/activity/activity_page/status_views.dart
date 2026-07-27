part of '../activity_page.dart';

class _LoadingSkeleton extends StatelessWidget {
  const _LoadingSkeleton();

  @override
  Widget build(BuildContext context) {
    return ListView.separated(
      padding: const EdgeInsets.all(Grid.gutter),
      itemCount: 8,
      separatorBuilder: (_, _) => const SizedBox(height: Grid.xs),
      itemBuilder: (context, _) => Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 36,
            height: 36,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: context.colors.outlineVariant.withValues(alpha: 0.4),
            ),
          ),
          const SizedBox(width: Grid.twelve),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Container(
                  width: 140,
                  height: 12,
                  decoration: BoxDecoration(
                    color: context.colors.outlineVariant.withValues(alpha: 0.4),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
                const SizedBox(height: Grid.xxs),
                Container(
                  width: double.infinity,
                  height: 10,
                  decoration: BoxDecoration(
                    color: context.colors.outlineVariant.withValues(alpha: 0.3),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
                const SizedBox(height: Grid.half),
                Container(
                  width: 220,
                  height: 10,
                  decoration: BoxDecoration(
                    color: context.colors.outlineVariant.withValues(alpha: 0.2),
                    borderRadius: BorderRadius.circular(4),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _EmptySurface extends StatelessWidget {
  final IconData icon;
  final String message;
  final String? detail;

  const _EmptySurface({required this.icon, required this.message, this.detail});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: Grid.lg, color: context.colors.onSurfaceVariant),
            const SizedBox(height: Grid.xxs),
            Text(
              message,
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            if (detail != null) ...[
              const SizedBox(height: Grid.half),
              Text(
                detail!,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
                textAlign: TextAlign.center,
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _EmptyFilterState extends StatelessWidget {
  final InboxFilter filter;
  final bool unreadOnly;

  const _EmptyFilterState({required this.filter, required this.unreadOnly});

  @override
  Widget build(BuildContext context) {
    if (unreadOnly) {
      return const _EmptySurface(
        icon: LucideIcons.mailOpen,
        message: 'No unread messages',
        detail: 'Turn off the unread filter to see read messages.',
      );
    }
    final (icon, message) = switch (filter) {
      InboxFilter.mention => (LucideIcons.atSign, 'No mentions yet'),
      InboxFilter.thread => (
        LucideIcons.messageSquare,
        'No thread replies yet',
      ),
      InboxFilter.needsAction => (
        LucideIcons.circleAlert,
        'Nothing needs your action',
      ),
      InboxFilter.activity => (
        LucideIcons.activity,
        'No recent channel activity',
      ),
      InboxFilter.agentActivity => (LucideIcons.bot, 'No agent updates'),
      _ => (LucideIcons.bell, 'No activity yet'),
    };
    return _EmptySurface(icon: icon, message: message);
  }
}

class _ErrorView extends StatelessWidget {
  final Future<void> Function() onRetry;

  const _ErrorView({required this.onRetry});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            LucideIcons.triangleAlert,
            size: Grid.lg,
            color: context.colors.error,
          ),
          const SizedBox(height: Grid.xxs),
          Text(
            'Failed to load activity',
            style: context.textTheme.bodyMedium?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: Grid.xs),
          FilledButton.icon(
            onPressed: () => unawaited(onRetry()),
            icon: const Icon(LucideIcons.refreshCcw, size: 16),
            label: const Text('Retry'),
          ),
        ],
      ),
    );
  }
}
