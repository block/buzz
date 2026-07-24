part of '../channels_page.dart';

class _ChannelsBody extends StatelessWidget {
  final List<Channel>? channels;
  final AsyncValue<List<Channel>> channelsAsync;
  final bool showError;
  final SessionState sessionState;
  final String relayHost;
  final bool showConnectionBanner;
  final String? currentPubkey;
  final Future<void> Function() onRefresh;
  final Future<void> Function() onRetryConnection;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _ChannelsBody({
    required this.channels,
    required this.channelsAsync,
    required this.showError,
    required this.sessionState,
    required this.relayHost,
    required this.showConnectionBanner,
    required this.currentPubkey,
    required this.onRefresh,
    required this.onRetryConnection,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context) {
    final barHeight = frostedAppBarHeight(context);

    if (channels != null) {
      final connectionLost = sessionState.status == SessionStatus.failed;
      final bannerVisible = connectionLost || showConnectionBanner;
      return Stack(
        children: [
          RefreshIndicator(
            edgeOffset: barHeight,
            onRefresh: onRefresh,
            child: CustomScrollView(
              slivers: [
                SliverToBoxAdapter(child: SizedBox(height: barHeight)),
                if (bannerVisible)
                  const SliverToBoxAdapter(
                    child: SizedBox(height: _kBannerHeight),
                  ),
                _SliverChannelsList(
                  channels: channels!,
                  currentPubkey: currentPubkey,
                  onSelectChannel: onSelectChannel,
                ),
              ],
            ),
          ),
          Positioned(
            top: barHeight,
            left: 0,
            right: 0,
            child: connectionLost
                ? _ConnectionLostBanner(
                    error: sessionState.lastError,
                    onRetry: onRetryConnection,
                  )
                : showConnectionBanner
                ? _ConnectionBanner(state: sessionState)
                : const SizedBox.shrink(),
          ),
        ],
      );
    }

    if (sessionState.status == SessionStatus.failed) {
      return Padding(
        padding: EdgeInsets.only(top: barHeight),
        child: _OfflineView(
          error: sessionState.lastError,
          relayHost: relayHost,
          onRetry: onRetryConnection,
        ),
      );
    }

    // The error view is gated on a grace timer in the parent — see the
    // useEffect in ChannelsPage. While the grace window is in flight we fall
    // through to the connection banner so transient relay-cancellation errors
    // don't flash the error UI.
    if (showError && channelsAsync.hasError) {
      return Padding(
        padding: EdgeInsets.only(top: barHeight),
        child: _ErrorView(error: channelsAsync.error!, onRetry: onRefresh),
      );
    }

    return Padding(
      padding: EdgeInsets.only(top: barHeight),
      child: _ConnectionBanner(
        state: SessionState(
          status: sessionState.status == SessionStatus.connected
              ? SessionStatus.connecting
              : sessionState.status,
          reconnectAttempt: sessionState.reconnectAttempt,
          lastError: sessionState.lastError,
        ),
      ),
    );
  }
}

class _SliverChannelsList extends HookConsumerWidget {
  final List<Channel> channels;
  final String? currentPubkey;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _SliverChannelsList({
    required this.channels,
    required this.currentPubkey,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final readState = ref.watch(readStateProvider);
    final sectionsState = ref.watch(channelSectionsProvider);
    final mutesState = ref.watch(channelMutesProvider);
    final mutedChannelIds = {
      for (final entry in mutesState.store.channels.entries)
        if (entry.value.muted) entry.key,
    };
    final starsState = ref.watch(channelStarsProvider);
    final starredChannelIds = {
      for (final entry in starsState.store.channels.entries)
        if (entry.value.starred) entry.key,
    };
    final visibleChannels = channels
        .where((channel) => channel.isMember && !channel.isArchived)
        .toList();
    final streamChannels = visibleChannels
        .where((channel) => channel.isStream)
        .toList();
    final dmChannels = sortDmChannelsByDisplayLabel(
      visibleChannels.where((channel) => channel.isDm),
      currentPubkey: currentPubkey,
    );

    final starredExpanded = useState(true);
    final channelsExpanded = useState(true);
    final dmsExpanded = useState(true);
    final initialSeedComplete = useState(false);
    final seededPubkey = useRef<String?>(null);
    final seedCompleteForPubkey =
        seededPubkey.value == readState.pubkey && initialSeedComplete.value;

    useEffect(() {
      if (!readState.isReady) {
        return null;
      }

      return deferReadStateUpdate(context, () {
        if (seededPubkey.value != readState.pubkey) {
          seededPubkey.value = readState.pubkey;
          initialSeedComplete.value = false;
        }

        if (initialSeedComplete.value) {
          return;
        }

        final notifier = ref.read(readStateProvider.notifier);
        for (final channel in visibleChannels) {
          if (readState.effectiveTimestamp(channel.id) != null) {
            continue;
          }

          final lastMessageAt = dateTimeToUnixSeconds(channel.lastMessageAt);
          if (lastMessageAt != null) {
            notifier.seedContextRead(channel.id, lastMessageAt);
          }
        }
        initialSeedComplete.value = true;
      });
    }, [readState.isReady, readState.pubkey, visibleChannels]);

    final unreadState = _computeUnreadChannelState(
      channels: visibleChannels,
      readState: readState,
      channelsNotifier: ref.read(channelsProvider.notifier),
    );
    final unreadChannelIds = {
      for (final channelId in unreadState.ids)
        if (seedCompleteForPubkey ||
            readState.effectiveTimestamp(channelId) != null)
          channelId,
    };
    final unreadChannelCounts = {
      for (final entry in unreadState.counts.entries)
        if (unreadChannelIds.contains(entry.key)) entry.key: entry.value,
    };

    // Build sorted user-defined sections and compute which stream channels
    // belong to each section. Channels not assigned to any valid section fall
    // through to the built-in "Channels" list.
    final userSections = sectionsState.store.sections.toList()
      ..sort((a, b) => a.order.compareTo(b.order));
    final sectionAssignments = sectionsState.store.assignments;
    final validSectionIds = {for (final s in userSections) s.id};
    final assignedChannelIds = {
      for (final entry in sectionAssignments.entries)
        if (validSectionIds.contains(entry.value)) entry.key,
    };
    // Starred is exclusive: a starred channel lives only in the Starred section,
    // not in its custom section or the default Channels list.
    final starredStreamChannels = streamChannels
        .where((c) => starredChannelIds.contains(c.id))
        .toList();
    final ungroupedStreamChannels = streamChannels
        .where(
          (c) =>
              !assignedChannelIds.contains(c.id) &&
              !starredChannelIds.contains(c.id),
        )
        .toList();

    final sectionExpandedStates = useState<Map<String, bool>>({});

    bool sectionExpanded(String sectionId) =>
        sectionExpandedStates.value[sectionId] ?? true;

    void toggleSection(String sectionId) {
      sectionExpandedStates.value = {
        ...sectionExpandedStates.value,
        sectionId: !sectionExpanded(sectionId),
      };
    }

    return SliverPadding(
      padding: const EdgeInsets.only(top: Grid.xxs, bottom: 80),
      sliver: SliverList.list(
        children: [
          if (visibleChannels.isEmpty)
            const _EmptyState()
          else ...[
            // Starred channels (exclusive — pinned above all sections).
            if (starredStreamChannels.isNotEmpty)
              _ChannelSection(
                title: 'Starred',
                icon: LucideIcons.star,
                showTopDivider: false,
                expanded: starredExpanded.value,
                onToggle: () => starredExpanded.value = !starredExpanded.value,
                channels: starredStreamChannels,
                unreadChannelIds: unreadChannelIds,
                unreadChannelCounts: unreadChannelCounts,
                mutedChannelIds: mutedChannelIds,
                currentPubkey: currentPubkey,
                emptyLabel: '',
                onSelectChannel: onSelectChannel,
              ),
            // User-defined sections for stream channels, in user-defined order.
            for (final section in userSections)
              _CustomChannelSection(
                section: section,
                channels: streamChannels
                    .where(
                      (c) =>
                          sectionAssignments[c.id] == section.id &&
                          !starredChannelIds.contains(c.id),
                    )
                    .toList(),
                unreadChannelIds: unreadChannelIds,
                unreadChannelCounts: unreadChannelCounts,
                mutedChannelIds: mutedChannelIds,
                currentPubkey: currentPubkey,
                expanded: sectionExpanded(section.id),
                isFirst: userSections.first.id == section.id,
                isLast: userSections.last.id == section.id,
                showTopDivider:
                    starredStreamChannels.isNotEmpty ||
                    userSections.first.id != section.id,
                onToggle: () => toggleSection(section.id),
                onRename: () async {
                  final name = await showDialog<String>(
                    context: context,
                    builder: (_) => _SectionNameDialog(
                      title: 'Rename Section',
                      confirmLabel: 'Rename',
                      initialValue: section.name,
                    ),
                  );
                  if (name != null && name.isNotEmpty) {
                    ref
                        .read(channelSectionsProvider.notifier)
                        .renameSection(section.id, name);
                  }
                },
                onDelete: () async {
                  final confirmed = await showDialog<bool>(
                    context: context,
                    builder: (_) => AlertDialog(
                      title: Text('Delete "${section.name}"?'),
                      content: const Text(
                        'Channels in this section will move back to the main list.',
                      ),
                      actions: [
                        TextButton(
                          onPressed: () => Navigator.pop(context, false),
                          child: const Text('Cancel'),
                        ),
                        TextButton(
                          onPressed: () => Navigator.pop(context, true),
                          child: Text(
                            'Delete',
                            style: TextStyle(color: context.colors.error),
                          ),
                        ),
                      ],
                    ),
                  );
                  if (confirmed == true) {
                    ref
                        .read(channelSectionsProvider.notifier)
                        .deleteSection(section.id);
                  }
                },
                onMoveUp: () => ref
                    .read(channelSectionsProvider.notifier)
                    .moveSectionUp(section.id),
                onMoveDown: () => ref
                    .read(channelSectionsProvider.notifier)
                    .moveSectionDown(section.id),
                onSelectChannel: onSelectChannel,
                onMarkChannelRead: (channel) {
                  final ts = dateTimeToUnixSeconds(channel.lastMessageAt);
                  if (ts != null) {
                    ref
                        .read(readStateProvider.notifier)
                        .markContextRead(channel.id, ts);
                    ref
                        .read(channelsProvider.notifier)
                        .clearObservedUnreadCoveredByRead(channel.id, ts);
                  }
                },
              ),
            _ChannelSection(
              title: 'Channels',
              icon: LucideIcons.hash,
              showTopDivider:
                  starredStreamChannels.isNotEmpty || userSections.isNotEmpty,
              expanded: channelsExpanded.value,
              onToggle: () => channelsExpanded.value = !channelsExpanded.value,
              channels: ungroupedStreamChannels,
              unreadChannelIds: unreadChannelIds,
              unreadChannelCounts: unreadChannelCounts,
              mutedChannelIds: mutedChannelIds,
              currentPubkey: currentPubkey,
              emptyLabel: 'No stream channels yet',
              onSelectChannel: onSelectChannel,
            ),
            _ChannelSection(
              title: 'DMs',
              icon: LucideIcons.messagesSquare,
              showTopDivider: true,
              expanded: dmsExpanded.value,
              onToggle: () => dmsExpanded.value = !dmsExpanded.value,
              channels: dmChannels,
              unreadChannelIds: unreadChannelIds,
              unreadChannelCounts: unreadChannelCounts,
              mutedChannelIds: mutedChannelIds,
              currentPubkey: currentPubkey,
              emptyLabel: 'No direct messages yet',
              onSelectChannel: onSelectChannel,
            ),
          ],
        ],
      ),
    );
  }
}
