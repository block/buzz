part of 'me_page.dart';

/// Joined Solo chats that are archived, most recently archived first.
/// Archiving hides a chat from Solo without deleting it.
List<Channel> archivedSoloChats(
  Iterable<Channel> channels,
  Iterable<String> legacyChannels,
) =>
    channels
        .where(
          (c) => c.isMember && c.isArchived && c.isSoloChat(legacyChannels),
        )
        .toList()
      ..sort((a, b) => b.archivedAt!.compareTo(a.archivedAt!));

/// Opens the archived Solo chats from the end of the Solo list.
class _ArchivedSoloChatsRow extends StatelessWidget {
  const _ArchivedSoloChatsRow({required this.count});
  final int count;

  @override
  Widget build(BuildContext context) {
    final color = channelNavigationForeground(context, unread: false);
    void open() => Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => const AdaptiveConversationDestination(
          child: ArchivedSoloChatsPage(),
        ),
      ),
    );
    return Semantics(
      label: 'Archived chats, $count',
      button: true,
      excludeSemantics: true,
      onTap: open,
      child: InkWell(
        key: const ValueKey('solo-archived-chats'),
        borderRadius: BorderRadius.circular(Radii.container),
        onTap: open,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(
            Grid.xxs,
            Grid.inventoryRowInset,
            Grid.xxs + Grid.half,
            Grid.inventoryRowInset,
          ),
          child: Row(
            children: [
              SizedBox(
                width: 24,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(BuzzIcons.archive, size: 20, color: color),
                ),
              ),
              const SizedBox(width: Grid.inventoryLabelGap),
              Expanded(
                child: Text(
                  'Archived',
                  style: navigationChatTitleTextStyle.copyWith(color: color),
                ),
              ),
              Text(
                '$count',
                style: navigationChatTitleTextStyle.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Archived Solo chats. Each opens read-only; its details offer Unarchive.
class ArchivedSoloChatsPage extends ConsumerWidget {
  const ArchivedSoloChatsPage({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final channels = ref.watch(channelsProvider);
    final legacyChannels = ref.watch(
      meGroupsProvider.select((g) => g.value?.channels ?? const <String>[]),
    );
    return FrostedScaffold(
      appBar: const FrostedAppBar(
        nativeTitle: 'Archived',
        title: Text('Archived'),
      ),
      body: channels.when(
        loading: () => const Center(child: BuzzLoadingIndicator()),
        error: (_, _) =>
            _MeRetry(onRetry: () => ref.invalidate(channelsProvider)),
        data: (all) {
          final archived = archivedSoloChats(all, legacyChannels);
          if (archived.isEmpty) {
            return const Center(child: Text('No archived chats'));
          }
          return ListView(
            padding: EdgeInsets.fromLTRB(
              Grid.gutter - Grid.xxs,
              frostedAppBarHeight(context) + Grid.xxs,
              Grid.gutter - Grid.xxs,
              MediaQuery.viewPaddingOf(context).bottom + Grid.gutter,
            ),
            children: [
              for (final channel in archived)
                SoloChatTile(key: ValueKey(channel.id), channel: channel),
            ],
          );
        },
      ),
    );
  }
}
