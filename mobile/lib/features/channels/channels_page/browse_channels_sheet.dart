part of '../channels_page.dart';

class _BrowseChannelsSheet extends HookConsumerWidget {
  const _BrowseChannelsSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channelsAsync = ref.watch(channelsProvider);
    final directoryState = ref.watch(channelDirectoryLoadStatusProvider);
    final activeDirectoryScope = channelDirectoryScope(
      ref.watch(relayConfigProvider).baseUrl,
      ref.watch(myPubkeyProvider),
    );
    final directoryStatus = directoryState.scope == activeDirectoryScope
        ? directoryState.status
        : ChannelDirectoryLoadStatus.idle;
    final channels = channelsAsync.asData?.value
        .where((channel) => channel.canJoin)
        .toList();
    channels?.sort(
      (left, right) =>
          left.name.toLowerCase().compareTo(right.name.toLowerCase()),
    );

    useEffect(() {
      unawaited(
        Future<void>.microtask(
          ref.read(channelsProvider.notifier).ensureDirectoryLoaded,
        ),
      );
      return null;
    }, const []);

    final directoryIsLoading =
        directoryStatus == ChannelDirectoryLoadStatus.idle ||
        directoryStatus == ChannelDirectoryLoadStatus.loading;
    final directoryHasError =
        directoryStatus == ChannelDirectoryLoadStatus.error ||
        channelsAsync.hasError;

    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          Grid.xs,
        ),
        child: CustomScrollView(
          shrinkWrap: true,
          slivers: [
            SliverToBoxAdapter(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Join an open channel to add it to your conversations.',
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                  const SizedBox(height: Grid.xs),
                ],
              ),
            ),
            if (directoryIsLoading && (channels == null || channels.isEmpty))
              const SliverToBoxAdapter(
                child: Padding(
                  padding: EdgeInsets.all(Grid.sm),
                  child: Center(child: BuzzLoadingIndicator()),
                ),
              )
            else if (directoryHasError &&
                (channels == null || channels.isEmpty))
              SliverToBoxAdapter(
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: Grid.sm),
                  child: Column(
                    children: [
                      Text(
                        'Couldn’t load open channels.',
                        textAlign: TextAlign.center,
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),
                      const SizedBox(height: Grid.xxs),
                      TextButton(
                        key: const Key('browse-channels-retry'),
                        onPressed: () => unawaited(
                          ref.read(channelsProvider.notifier).retryDirectory(),
                        ),
                        child: const Text('Try again'),
                      ),
                    ],
                  ),
                ),
              )
            else if (channels == null || channels.isEmpty)
              SliverToBoxAdapter(
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: Grid.sm),
                  child: Text(
                    'No open channels available to join.',
                    textAlign: TextAlign.center,
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ),
              )
            else
              SliverList.builder(
                itemCount: channels.length,
                itemBuilder: (context, index) => _JoinableChannelTile(
                  channel: channels[index],
                  closeAfterJoin: true,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _JoinableChannelTile extends HookConsumerWidget {
  final Channel channel;
  final bool closeAfterJoin;

  const _JoinableChannelTile({
    required this.channel,
    required this.closeAfterJoin,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final isJoining = useState(false);
    final actionError = useState<String?>(null);

    Future<void> join() async {
      if (isJoining.value) return;
      isJoining.value = true;
      actionError.value = null;
      try {
        await ref.read(channelActionsProvider).joinChannel(channel.id);
        if (closeAfterJoin && context.mounted) Navigator.of(context).pop();
      } catch (error) {
        actionError.value = error.toString();
      } finally {
        isJoining.value = false;
      }
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        ListTile(
          key: Key('browse-channel-${channel.id}'),
          contentPadding: EdgeInsets.zero,
          leading: Icon(channelIcon(channel)),
          title: Text(channel.name),
          subtitle: channel.description.trim().isEmpty
              ? null
              : Text(
                  channel.description,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                ),
          trailing: FilledButton.tonal(
            key: Key('browse-channel-join-${channel.id}'),
            onPressed: isJoining.value ? null : () => unawaited(join()),
            child: Text(isJoining.value ? 'Joining…' : 'Join'),
          ),
        ),
        if (actionError.value case final error?)
          Align(
            alignment: Alignment.centerLeft,
            child: Text(
              error,
              key: Key('browse-channel-error-${channel.id}'),
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.error,
              ),
            ),
          ),
      ],
    );
  }
}
