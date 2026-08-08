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

class _MessageTimelineSkeleton extends StatelessWidget {
  final double appBarTitleContentHeight;
  final SessionStatus status;

  const _MessageTimelineSkeleton({
    required this.appBarTitleContentHeight,
    required this.status,
  });

  @override
  Widget build(BuildContext context) {
    final semanticsLabel = switch (status) {
      SessionStatus.connecting => 'Connecting',
      SessionStatus.reconnecting => 'Reconnecting',
      SessionStatus.connected || SessionStatus.disconnected => 'Loading',
    };
    return Semantics(
      key: const Key('channel-detail-connection-skeleton'),
      liveRegion: true,
      label: semanticsLabel,
      child: ExcludeSemantics(
        child: ListView.separated(
          physics: const NeverScrollableScrollPhysics(),
          padding: EdgeInsets.fromLTRB(
            Grid.gutter,
            frostedAppBarHeight(
                  context,
                  titleContentHeight: appBarTitleContentHeight,
                ) +
                Grid.xs,
            Grid.gutter,
            Grid.xs,
          ),
          itemCount: 4,
          separatorBuilder: (_, _) => const SizedBox(height: Grid.xs),
          itemBuilder: (_, index) => _MessageSkeletonRow(index: index),
        ),
      ),
    );
  }
}

class _MessageSkeletonRow extends StatelessWidget {
  final int index;

  const _MessageSkeletonRow({required this.index});

  @override
  Widget build(BuildContext context) {
    const authorWidths = <double>[112, 96, 128, 80];
    const lineWidths = <List<double>>[
      [280, 224],
      [272, 184],
      [232],
      [288, 216],
    ];
    final availableWidth = MediaQuery.sizeOf(context).width - 88;

    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SkeletonBar(
          width: 36,
          height: 36,
          borderRadius: BorderRadius.circular(Radii.full),
        ),
        const SizedBox(width: Grid.xxs),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  SkeletonBar(width: authorWidths[index], height: 15),
                  const SizedBox(width: Grid.xxs),
                  const SkeletonBar(width: 40, height: 12),
                ],
              ),
              const SizedBox(height: Grid.half),
              for (final width in lineWidths[index]) ...[
                SkeletonBar(width: min(width, availableWidth), height: 16),
                const SizedBox(height: Grid.half),
              ],
              const Row(
                children: [
                  SkeletonBar(width: 32, height: 16),
                  SizedBox(width: Grid.xs),
                  SkeletonBar(width: 32, height: 16),
                  SizedBox(width: Grid.xs),
                  SkeletonBar(width: 32, height: 16),
                ],
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _ForumConnectionSkeleton extends StatelessWidget {
  final SessionStatus status;

  const _ForumConnectionSkeleton({required this.status});

  @override
  Widget build(BuildContext context) {
    final semanticsLabel = switch (status) {
      SessionStatus.connecting => 'Connecting',
      SessionStatus.reconnecting => 'Reconnecting',
      SessionStatus.connected || SessionStatus.disconnected => 'Loading',
    };
    return Semantics(
      key: const Key('forum-connection-skeleton'),
      liveRegion: true,
      label: semanticsLabel,
      child: ExcludeSemantics(
        child: IgnorePointer(
          child: ExcludeFocus(
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: context.colors.surface,
                borderRadius: BorderRadius.circular(Radii.lg),
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: Padding(
                padding: const EdgeInsets.all(Grid.twelve),
                child: SkeletonShimmer(
                  child: const Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Row(
                        children: [
                          SkeletonBar(
                            width: 28,
                            height: 28,
                            borderRadius: BorderRadius.all(
                              Radius.circular(Radii.full),
                            ),
                          ),
                          SizedBox(width: Grid.xxs),
                          SkeletonBar(width: 112, height: 14),
                        ],
                      ),
                      SizedBox(height: Grid.xxs),
                      SkeletonBar(width: 240, height: 14),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
