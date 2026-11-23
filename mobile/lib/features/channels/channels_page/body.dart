part of '../channels_page.dart';

class _ChannelsBody extends StatelessWidget {
  final Widget? soloSliver;
  final VoidCallback? onInbox;
  final List<Channel>? channels;
  final AsyncValue<List<Channel>> channelsAsync;
  final bool showError;
  final SessionStatus sessionStatus;
  final ValueChanged<bool> onReadyChanged;
  final String? currentPubkey;
  final double topSectionHeight;
  final bool usesPinnedGradient;
  final ScrollController scrollController;
  final Future<void> Function() onRefresh;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _ChannelsBody({
    this.soloSliver,
    required this.onInbox,
    required this.channels,
    required this.channelsAsync,
    required this.showError,
    required this.sessionStatus,
    required this.onReadyChanged,
    required this.currentPubkey,
    required this.topSectionHeight,
    required this.usesPinnedGradient,
    required this.scrollController,
    required this.onRefresh,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context) {
    final inbox = Semantics(
      label: 'Activity',
      button: true,
      onTap: onInbox,
      excludeSemantics: true,
      child: InkWell(
        borderRadius: BorderRadius.circular(Radii.md),
        onTap: onInbox,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: _kChannelSectionInset,
            vertical: _kChannelRowVerticalPadding,
          ),
          child: Row(
            children: [
              SizedBox(
                width: _kChannelLeadingWidth,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(
                    BuzzIcons.bell,
                    size: _kChannelIconSize,
                    color: navigationPrimaryForeground(context),
                  ),
                ),
              ),
              const SizedBox(width: _kChannelLabelGap),
              Expanded(
                child: Text(
                  'Activity',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: navigationSectionTextStyle.copyWith(
                    fontSize: 17,
                    height: 22 / 17,
                    color: navigationPrimaryForeground(context),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
    final barHeight = topSectionHeight;
    final loadedChannels = channels;
    final loading = loadedChannels == null && !showError;
    final content = showError && channelsAsync.hasError
        ? Padding(
            padding: EdgeInsets.only(top: barHeight),
            child: _ErrorView(error: channelsAsync.error!, onRetry: onRefresh),
          )
        : loadedChannels == null
        ? const SizedBox.shrink()
        : BeeRefreshIndicator(
            edgeOffset: barHeight,
            onRefresh: onRefresh,
            child: CustomScrollView(
              controller: scrollController,
              // Transparent list gaps must remain hit-testable so a new drag
              // can interrupt ballistic scrolling. The app bar is painted
              // later and retains its community and profile controls.
              hitTestBehavior: HitTestBehavior.translucent,
              slivers: [
                SliverToBoxAdapter(child: SizedBox(height: barHeight)),
                const SliverToBoxAdapter(child: _ReadStateSyncNotice()),
                if (onInbox != null)
                  SliverPadding(
                    padding: const EdgeInsets.only(top: Grid.twelve),
                    sliver: SliverToBoxAdapter(child: inbox),
                  ),
                usesPinnedGradient
                    ? _SliverChannelsList(
                        channels: loadedChannels,
                        currentPubkey: currentPubkey,
                        onSelectChannel: onSelectChannel,
                        soloSliver: soloSliver,
                      )
                    : DecoratedSliver(
                        decoration: BoxDecoration(
                          color: context.colors.surface,
                          borderRadius: const BorderRadius.vertical(
                            top: Radius.circular(Radii.dialog),
                          ),
                        ),
                        sliver: _SliverChannelsList(
                          channels: loadedChannels,
                          currentPubkey: currentPubkey,
                          onSelectChannel: onSelectChannel,
                          soloSliver: soloSliver,
                        ),
                      ),
                SliverToBoxAdapter(
                  child: SizedBox(height: MediaQuery.paddingOf(context).bottom),
                ),
              ],
            ),
          );

    final reveal = SkeletonReveal(
      onReadyChanged: onReadyChanged,
      loading: loading,
      loadingSemanticsKey: const Key('channels-connection-skeleton'),
      loadingLabel: switch (sessionStatus) {
        SessionStatus.connecting => 'Connecting',
        SessionStatus.reconnecting => 'Reconnecting',
        _ => 'Loading',
      },
      shimmerEnabled: sessionStatus != SessionStatus.disconnected,
      skeleton: _ChannelsSkeleton(
        channels: loadedChannels,
        topInset:
            barHeight + (onInbox != null && loadedChannels == null ? 56 : 0),
      ),
      content: content,
    );
    return Stack(
      children: [
        reveal,
        if (onInbox != null &&
            (loadedChannels == null || (showError && channelsAsync.hasError)))
          Positioned(top: barHeight, left: 0, right: 0, child: inbox),
      ],
    );
  }
}

class _SliverChannelsList extends HookConsumerWidget {
  final List<Channel> channels;
  final String? currentPubkey;
  final Future<void> Function(Channel channel) onSelectChannel;

  /// Home's Solo chats, rendered at the Solo section's place in the order.
  /// Without it, Solo chats stay out of this list.
  final Widget? soloSliver;

  const _SliverChannelsList({
    required this.channels,
    required this.currentPubkey,
    required this.onSelectChannel,
    this.soloSliver,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final readState = ref.watch(readStateProvider);
    final sectionsState = ref.watch(channelSectionsProvider);
    final layout = ref.watch(sidebarLayoutProvider);
    final legacySoloIds = ref.watch(
      meGroupsProvider.select((g) => g.value?.channels ?? const <String>[]),
    );
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
    bool isSoloRow(Channel channel) => channel.isSoloChat(legacySoloIds);
    // Team conversations own this list's read-state seeding and highlights;
    // Solo rows keep Solo's own unread tracking.
    final visibleChannels = channels
        .where(
          (channel) =>
              channel.isMember && !channel.isArchived && !isSoloRow(channel),
        )
        .toList();
    final soloChannels = soloSliver == null
        ? const <Channel>[]
        : channels
              .where((c) => c.isMember && !c.isArchived && isSoloRow(c))
              .toList();
    // Place each row once, as desktop's sidebar does: Starred, then its
    // workspace, then its home section. Mobile doesn't show forums.
    final sectionKeyOf = <String, String>{};
    final buckets = <String, List<Channel>>{};
    for (final channel in [...visibleChannels, ...soloChannels]) {
      if (!channel.isStream && !channel.isDm) continue;
      final solo = isSoloRow(channel);
      final key = layout.sectionKeyFor(
        channel.id,
        starred: starredChannelIds.contains(channel.id),
        home: solo
            ? 'solo'
            : channel.isDm
            ? 'dms'
            : 'channels',
      );
      sectionKeyOf[channel.id] = key;
      // Solo's own section lists its chats itself.
      if (key != 'solo') (buckets[key] ??= []).add(channel);
    }
    final unsortedDms = buckets['dms'] ?? const <Channel>[];
    // Rebuild only when the DM order changes, not for every profile fetch.
    final dmOrder = ref.watch(
      identityNameSourcesProvider.select(
        (names) => [
          for (final channel in sortDmChannelsByDisplayLabel(
            unsortedDms,
            currentPubkey: currentPubkey,
            names: names,
          ))
            channel.id,
        ].join('\u0000'),
      ),
    );
    final dmRank = {
      for (final (index, id) in dmOrder.split('\u0000').indexed) id: index,
    };
    final dmChannels = unsortedDms.toList()
      ..sort((a, b) => dmRank[a.id]!.compareTo(dmRank[b.id]!));

    final sortState = ref.watch(channelSortProvider);
    final filters = ref.watch(channelFilterProvider);
    useListenable(filters);
    final selectedChannel = useState<String?>(null);
    final clock = useState(0);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final activity = <String, AsyncValue<int?>>{};

    var nextExpiry = 0;
    for (final MapEntry(key: id, value: key) in sectionKeyOf.entries) {
      final group = sidebarGroupKey(key);
      if (filters?.store.modeFor(group) != ChannelFilterMode.active) continue;
      final evidence = ref.watch(channelFilterActivityProvider(id));
      activity[id] = evidence;
      // Solo uses the same evidence and handles its own row expiry.
      if (key == 'solo') continue;
      final at = evidence.asData?.value;
      if (at == null) continue;
      final expiry = at + filters!.store.windowFor(group);
      if (expiry > now && (nextExpiry == 0 || expiry < nextExpiry)) {
        nextExpiry = expiry;
      }
    }
    useEffect(() {
      if (nextExpiry == 0) return null;
      final timer = Timer(
        Duration(seconds: nextExpiry - now),
        () => clock.value++,
      );
      return timer.cancel;
    }, [nextExpiry, clock.value]);
    Future<void> openFilteredChannel(Channel channel) async {
      selectedChannel.value = channel.id;
      try {
        await onSelectChannel(channel);
      } finally {
        if (context.mounted) selectedChannel.value = null;
      }
    }

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

        final seeds = <String, int>{};
        for (final channel in visibleChannels) {
          if (readState.contexts.containsKey(channel.id)) {
            continue;
          }

          final lastMessageAt = dateTimeToUnixSeconds(channel.lastMessageAt);
          if (lastMessageAt != null) {
            seeds[channel.id] = lastMessageAt;
          }
        }
        ref.read(readStateProvider.notifier).seedContextsRead(seeds);
        initialSeedComplete.value = true;
      });
    }, [readState.isReady, readState.pubkey, visibleChannels]);

    final highlights = computeChannelListHighlights(
      channels: visibleChannels,
      readState: readState,
      channelsNotifier: ref.read(channelsProvider.notifier),
    );
    bool seeded(String channelId) =>
        seedCompleteForPubkey ||
        readState.effectiveTimestamp(channelId) != null;
    final locallyForcedChannelIds = readState.locallyForcedChannelIds;
    final unreadChannelIds = highlights.unread
        .where((id) => seeded(id) || locallyForcedChannelIds.contains(id))
        .toSet();
    final rowUnreadChannelIds = unreadChannelIds.intersection(
      highlights.rowUnread,
    );
    final attentionChannelIds = highlights.attention.where(seeded).toSet();
    // Solo rows show Solo's own unread state; filters must agree with it.
    final soloUnreadIds = soloSliver == null
        ? const <String>{}
        : ref.watch(meUnreadChannelIdsProvider);

    final liveSectionIds = [for (final s in layout.sections) s.id];
    // Desktop's unified sort record is read-only here.
    ValueChanged<ChannelSortMode>? sortSetter(String groupKey) =>
        sortState.editable
        ? (mode) => ref
              .read(channelSortProvider.notifier)
              .setSortModeFor(groupKey, mode, liveSectionIds: liveSectionIds)
        : null;

    final sectionExpandedStates = useState<Map<String, bool>>({});

    bool sectionExpanded(String key) =>
        sectionExpandedStates.value[key] ?? true;

    void toggleSection(String key) {
      sectionExpandedStates.value = {
        ...sectionExpandedStates.value,
        key: !sectionExpanded(key),
      };
    }

    bool visible(String group, Channel channel) {
      final evidence = activity[channel.id];
      return channelMatchesFilter(
        mode: filters?.store.modeFor(group) ?? ChannelFilterMode.all,
        unread: isSoloRow(channel)
            ? soloUnreadIds.contains(channel.id)
            : rowUnreadChannelIds.contains(channel.id) ||
                  attentionChannelIds.contains(channel.id),
        activitySeconds: evidence?.asData?.value,
        activityKnown: evidence?.asData != null,
        nowSeconds: now,
        windowSeconds:
            filters?.store.windowFor(group) ?? defaultActiveWindowSeconds,
        selected: selectedChannel.value == channel.id,
      );
    }

    // Only the team sections record can be edited, and only until desktop's
    // unified workspaces exist. The mobile display order is stored separately.
    final teamSections = sectionsState.editable
        ? (sectionsState.store.sections.toList()
            ..sort((a, b) => a.order.compareTo(b.order)))
        : const <ChannelSection>[];
    final teamSectionIds = {for (final s in teamSections) s.id};
    // A Team workspace's layout ID differs from its section ID when a Solo
    // project shares that ID.
    String? teamIdOf(String key) => switch (layout.section(key.substring(6))) {
      WorkspaceSection(:final teamId?) when teamSectionIds.contains(teamId) =>
        teamId,
      _ => null,
    };
    Widget row(Channel channel, String key) => isSoloRow(channel)
        ? Padding(
            key: ValueKey(channel.id),
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.gutter - Grid.xxs,
            ),
            child: SoloChatTile(channel: channel),
          )
        : _ChannelTile(
            key: ValueKey(channel.id),
            channel: channel,
            isUnread: unreadChannelIds.contains(channel.id),
            boldUnread: rowUnreadChannelIds.contains(channel.id),
            hasAttention: attentionChannelIds.contains(channel.id),
            isMuted: mutedChannelIds.contains(channel.id),
            currentPubkey: currentPubkey,
            onTap: () => openFilteredChannel(channel),
            sectionId: key.startsWith('group:') ? teamIdOf(key) : null,
          );

    // [layoutId] keys the workspace's saved filter; it differs from the Team
    // section's ID when a Solo project shares that ID.
    _SectionEdits editsFor(ChannelSection section, String layoutId) => (
      onRename: () async {
        final config = ref.read(relayConfigProvider);
        final edit = await showSectionRenameEditor(context, section);
        if (!context.mounted || edit == null) return;
        final current = ref.read(relayConfigProvider);
        if (current.storedOrigin != config.storedOrigin ||
            current.nsec != config.nsec) {
          return;
        }
        ref
            .read(channelSectionsProvider.notifier)
            .renameSection(
              section.id,
              edit.name == section.name.trim() ? null : edit.name,
              icon: edit.icon == section.icon ? null : edit.icon,
            );
      },
      onDelete: () async {
        final confirmed = await showBuzzDialog<bool>(
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
        // Desktop can lock legacy edits while the confirmation is open; the
        // section and its saved filter then both stay.
        if (confirmed == true &&
            context.mounted &&
            ref.read(channelSectionsProvider).editable) {
          // Local section removal must work without filter-read/write
          // authority. Cleanup is optional for this never-reused ID.
          ref.read(channelSectionsProvider.notifier).deleteSection(section.id);
          try {
            await filters?.clearDeletedSection(layoutId);
          } catch (_) {
            if (context.mounted) {
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(
                  content: Text(
                    'Section deleted. Its saved filter could not be cleared.',
                  ),
                ),
              );
            }
          }
        }
      },
    );

    Widget? sectionFor(String key) {
      final rows = buckets[key] ?? const <Channel>[];
      final group = sidebarGroupKey(key);
      ChannelSortMode mode() => sortState.sortModeFor(group);
      if (key == 'solo') return soloSliver;
      if (key == 'starred' || key == 'channels' || key == 'dms') {
        if (key == 'starred' && rows.isEmpty) return null;
        return _ChannelSection(
          key: ValueKey(key),
          title: switch (key) {
            'starred' => 'Starred',
            'channels' => 'Team',
            _ => 'Direct messages',
          },
          group: group,
          expanded: sectionExpanded(key),
          onToggle: () => toggleSection(key),
          // DMs default to the display-label alphabetical order (labels can
          // differ from channel names); Recent mode reorders by last message.
          channels: key == 'dms' && mode() == ChannelSortMode.alpha
              ? dmChannels
              : sortChannelsForList(rows, mode()),
          isVisible: (c) => visible(group, c),
          buildRow: (c) => row(c, key),
          emptyLabel: switch (key) {
            'channels' => 'No stream channels yet',
            'dms' => 'No direct messages yet',
            _ => '',
          },
          sortMode: mode(),
          onSortModeChange: sortSetter(group),
        );
      }
      final section = layout.section(key.substring(6));
      if (section == null) return null;
      final teamId = teamIdOf(key);
      final team = teamSections.where((s) => s.id == teamId).firstOrNull;
      return _CustomChannelSection(
        key: ValueKey(section.id),
        section: section,
        channels: sortChannelsForList(rows, mode()),
        isVisible: (c) => visible(group, c),
        buildRow: (c) => row(c, key),
        expanded: sectionExpanded(section.id),
        onToggle: () => toggleSection(section.id),
        edits: team == null ? null : editsFor(team, section.id),
        sortMode: mode(),
        onSortModeChange: sortSetter(group),
      );
    }

    final keys = layout.sectionKeys.where((key) => key != 'forums').toList();
    final orderKey = ref.watch(mobileSectionOrderKeyProvider);
    final saved = orderKey == null
        ? const <String>[]
        : ref.watch(mobileSectionOrderProvider(orderKey)).value ??
              const <String>[];
    final labels = <String, String>{};
    final sections = <Widget>[];
    for (final key in resolveMobileSectionOrder(keys, saved)) {
      final section = sectionFor(key);
      if (section == null) continue;
      sections.add(section);
      labels[key] = switch (key) {
        'starred' => 'Starred',
        'solo' => 'Solo',
        'channels' => 'Team',
        'dms' => 'Direct messages',
        _ => layout.section(key.substring(6))!.name,
      };
    }
    final empty = sectionKeyOf.isEmpty && layout.sections.isEmpty;
    final inventory = SliverMainAxisGroup(
      slivers: [
        if (empty) ...[
          ?soloSliver,
          const SliverToBoxAdapter(child: _EmptyState()),
        ] else
          for (final (index, section) in sections.indexed) ...[
            if (index > 0)
              const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
            section,
          ],
      ],
    );
    final content = _ActivityRecovery(activity: activity, child: inventory);
    if (orderKey == null) return content;
    return SectionOrderScope(
      labels: labels,
      defaultOrder: keys,
      storageKey: orderKey,
      child: content,
    );
  }
}

/// Says when unread status is not syncing with other devices: retries on tap
/// while connected, says it resumes on reconnect while offline, or, once the
/// relay refused read state for the session, says how to recover.
class _ReadStateSyncNotice extends ConsumerWidget {
  const _ReadStateSyncNotice();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final (:failed, :unsupported) = ref.watch(
      readStateProvider.select(
        (state) =>
            (failed: state.syncFailed, unsupported: state.syncUnsupported),
      ),
    );
    final online = ref.watch(
      relaySessionProvider.select(
        (session) => session.status == SessionStatus.connected,
      ),
    );
    // Unsupported outlasts unrelated successes: marks stay unsendable until
    // a new session, so it does not depend on the current error.
    if (!failed && !unsupported) return const SizedBox.shrink();
    final text = Text(
      unsupported
          ? 'Unread status can’t sync with your other devices right now. Sign in again or restart Buzz to try again.'
          : online
          ? 'Unread status isn’t syncing with your other devices. Tap to retry.'
          // Reconnecting resyncs read state on its own.
          : 'Unread status isn’t syncing with your other devices. It will retry when you’re back online.',
      style: context.textTheme.bodySmall?.copyWith(
        color: context.colors.onSurfaceVariant,
      ),
    );
    if (unsupported || !online) {
      return Padding(
        key: const Key('read-state-sync-notice'),
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xs,
          Grid.gutter,
          0,
        ),
        child: Padding(padding: const EdgeInsets.all(Grid.xs), child: text),
      );
    }
    return Padding(
      padding: const EdgeInsets.fromLTRB(Grid.gutter, Grid.xs, Grid.gutter, 0),
      child: Semantics(
        button: true,
        child: InkWell(
          key: const Key('read-state-sync-notice'),
          borderRadius: BorderRadius.circular(Radii.lg),
          onTap: () =>
              unawaited(ref.read(readStateProvider.notifier).retrySync()),
          child: Padding(padding: const EdgeInsets.all(Grid.xs), child: text),
        ),
      ),
    );
  }
}
