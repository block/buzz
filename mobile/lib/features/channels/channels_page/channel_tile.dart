part of '../channels_page.dart';

class _ChannelTile extends ConsumerWidget {
  final Channel channel;
  final int? unreadCount;
  final bool isUnread;
  final bool isMuted;
  final String? currentPubkey;
  final VoidCallback onTap;

  /// Called when the user requests to mark this channel read (from long-press
  /// actions menu). Null for channels in built-in sections.
  final VoidCallback? onMarkRead;

  /// The user-defined section this channel currently belongs to, or null.
  final String? sectionId;

  const _ChannelTile({
    required this.channel,
    this.unreadCount,
    required this.isUnread,
    required this.currentPubkey,
    required this.onTap,
    this.isMuted = false,
    this.onMarkRead,
    this.sectionId,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return InkWell(
      borderRadius: BorderRadius.circular(Radii.md),
      onTap: onTap,
      onLongPress: () => _showChannelActions(context, ref),
      child: Padding(
        padding: const EdgeInsets.only(
          left: _kChannelSectionInset,
          right: _kChannelSectionInset,
          top: _kChannelRowVerticalPadding,
          bottom: _kChannelRowVerticalPadding,
        ),
        child: Row(
          children: [
            if (channel.isDm)
              _DmAvatar(channel: channel, currentPubkey: currentPubkey)
            else
              SizedBox(
                width: _kChannelLeadingWidth,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Icon(
                    channelIcon(channel),
                    size: _kChannelIconSize,
                    color: context.colors.onSurface,
                  ),
                ),
              ),
            const SizedBox(width: _kChannelLabelGap),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    resolveDmChannelDisplayLabel(
                      channel,
                      currentPubkey: currentPubkey,
                    ),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.bodyMedium?.copyWith(
                      color: context.colors.onSurface,
                      fontWeight: isUnread ? FontWeight.w700 : FontWeight.w400,
                    ),
                  ),
                ],
              ),
            ),
            if (channel.isEphemeral) ...[
              const SizedBox(width: Grid.xxs),
              _EphemeralBadge(channel: channel),
            ],
            if (isMuted) ...[
              const SizedBox(width: Grid.xxs),
              Icon(
                LucideIcons.bellOff,
                size: 12,
                color: context.colors.onSurfaceVariant,
              ),
            ],
            if (isUnread && !channel.isDm) ...[
              const SizedBox(width: Grid.xxs),
              _UnreadBadge(channelId: channel.id, count: unreadCount ?? 0),
            ],
            if (!channel.isMember && !channel.isDm)
              Padding(
                padding: const EdgeInsets.only(right: Grid.xxs),
                child: Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.half + 2,
                    vertical: 3,
                  ),
                  decoration: BoxDecoration(
                    color: context.colors.primary.withValues(alpha: 0.1),
                    borderRadius: BorderRadius.circular(Radii.sm),
                  ),
                  child: Text(
                    'Open',
                    style: context.textTheme.labelSmall?.copyWith(
                      color: context.colors.primary,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }

  void _showChannelActions(BuildContext context, WidgetRef ref) {
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) {
        final sections = ref.read(channelSectionsProvider).store.sections
          ..sort((a, b) => a.order.compareTo(b.order));
        final isStarred =
            ref
                .read(channelStarsProvider)
                .store
                .channels[channel.id]
                ?.starred ==
            true;

        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                ListTile(
                  leading: Icon(
                    isStarred ? LucideIcons.starOff : LucideIcons.star,
                  ),
                  title: Text(isStarred ? 'Unstar channel' : 'Star channel'),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    if (isStarred) {
                      ref
                          .read(channelStarsProvider.notifier)
                          .unstarChannel(channel.id);
                    } else {
                      ref
                          .read(channelStarsProvider.notifier)
                          .starChannel(channel.id);
                    }
                  },
                ),
                ListTile(
                  leading: const Icon(LucideIcons.folderInput),
                  title: const Text('Move to section'),
                  onTap: () async {
                    Navigator.of(sheetContext).pop();
                    await _showMoveSectionSheet(context, ref, sections);
                  },
                ),
                ListTile(
                  leading: Icon(
                    isMuted ? LucideIcons.bell : LucideIcons.bellOff,
                  ),
                  title: Text(isMuted ? 'Unmute channel' : 'Mute channel'),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    if (isMuted) {
                      ref
                          .read(channelMutesProvider.notifier)
                          .unmuteChannel(channel.id);
                    } else {
                      ref
                          .read(channelMutesProvider.notifier)
                          .muteChannel(channel.id);
                    }
                  },
                ),
                ListTile(
                  leading: Icon(
                    isUnread ? LucideIcons.checkCheck : LucideIcons.circleDot,
                  ),
                  title: Text(isUnread ? 'Mark as read' : 'Mark as unread'),
                  onTap: () {
                    Navigator.of(sheetContext).pop();
                    final ts = dateTimeToUnixSeconds(channel.lastMessageAt);
                    if (ts != null) {
                      if (isUnread) {
                        onMarkRead?.call();
                        ref
                            .read(readStateProvider.notifier)
                            .markContextRead(channel.id, ts);
                        ref
                            .read(channelsProvider.notifier)
                            .clearObservedUnreadCoveredByRead(channel.id, ts);
                      } else {
                        ref
                            .read(readStateProvider.notifier)
                            .markContextUnread(channel.id);
                      }
                    }
                  },
                ),
              ],
            ),
          ),
        );
      },
    );
  }

  Future<void> _showMoveSectionSheet(
    BuildContext context,
    WidgetRef ref,
    List<ChannelSection> sections,
  ) async {
    await showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (sheetContext) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final section in sections)
                  ListTile(
                    leading: Icon(
                      LucideIcons.folder,
                      color: sectionId == section.id
                          ? Theme.of(sheetContext).colorScheme.primary
                          : null,
                    ),
                    title: Text(section.name),
                    trailing: sectionId == section.id
                        ? Icon(
                            LucideIcons.check,
                            color: Theme.of(sheetContext).colorScheme.primary,
                          )
                        : null,
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      ref
                          .read(channelSectionsProvider.notifier)
                          .assignChannel(channel.id, section.id);
                    },
                  ),
                ListTile(
                  leading: const Icon(LucideIcons.folderPlus),
                  title: const Text('New section…'),
                  onTap: () async {
                    Navigator.of(sheetContext).pop();
                    if (!context.mounted) return;
                    final name = await showDialog<String>(
                      context: context,
                      builder: (_) => const _SectionNameDialog(
                        title: 'New Section',
                        confirmLabel: 'Create',
                      ),
                    );
                    if (name != null && name.isNotEmpty) {
                      ref
                          .read(channelSectionsProvider.notifier)
                          .createSection(name);
                      // Assign after create — sections list has been mutated,
                      // re-read to find the new section by name.
                      final newSection = ref
                          .read(channelSectionsProvider)
                          .store
                          .sections
                          .lastWhere(
                            (s) => s.name == name.trim(),
                            orElse: () => const ChannelSection(
                              id: '',
                              name: '',
                              order: -1,
                            ),
                          );
                      if (newSection.id.isNotEmpty) {
                        ref
                            .read(channelSectionsProvider.notifier)
                            .assignChannel(channel.id, newSection.id);
                      }
                    }
                  },
                ),
                if (sectionId != null)
                  ListTile(
                    leading: const Icon(LucideIcons.folderMinus),
                    title: const Text('Remove from section'),
                    onTap: () {
                      Navigator.of(sheetContext).pop();
                      ref
                          .read(channelSectionsProvider.notifier)
                          .unassignChannel(channel.id);
                    },
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _DmAvatar extends ConsumerWidget {
  final Channel channel;
  final String? currentPubkey;

  const _DmAvatar({required this.channel, required this.currentPubkey});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final profiles = ref.watch(userCacheProvider);
    final presenceMap = ref.watch(presenceCacheProvider);
    final normalizedCurrent = currentPubkey?.toLowerCase();
    final otherPubkeys = [
      for (final pk in channel.participantPubkeys)
        if (pk.toLowerCase() != normalizedCurrent) pk.toLowerCase(),
    ];
    final visiblePubkeys = otherPubkeys.isNotEmpty
        ? otherPubkeys
        : channel.participantPubkeys.map((pk) => pk.toLowerCase()).toList();

    if (visiblePubkeys.length > 1) {
      return Container(
        width: 22,
        height: 22,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: context.colors.surfaceContainerHighest,
          shape: BoxShape.circle,
          border: Border.all(color: context.colors.outlineVariant),
        ),
        child: Text(
          '${visiblePubkeys.length}',
          style: context.textTheme.labelSmall?.copyWith(
            fontSize: 10,
            color: context.colors.onSurface,
            fontWeight: FontWeight.w600,
            height: 1,
          ),
        ),
      );
    }

    final otherPubkey = visiblePubkeys.isNotEmpty ? visiblePubkeys.first : null;
    final profile = otherPubkey != null ? profiles[otherPubkey] : null;

    // Trigger fetches if not cached yet.
    if (otherPubkey != null) {
      if (profile == null) {
        ref.read(userCacheProvider.notifier).preload([otherPubkey]);
      }
      ref.read(presenceCacheProvider.notifier).track([otherPubkey]);
    }

    final avatarUrl = profile?.avatarUrl;
    final initial =
        profile?.initial ??
        (channel.participants.isNotEmpty
            ? channel.participants.first[0].toUpperCase()
            : '?');
    final presence = otherPubkey != null
        ? (presenceMap[otherPubkey] ?? 'offline')
        : 'offline';

    return SizedBox(
      width: 22,
      height: 22,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          CircleAvatar(
            radius: 10,
            backgroundColor: context.colors.primaryContainer,
            backgroundImage: avatarUrl != null ? NetworkImage(avatarUrl) : null,
            child: avatarUrl == null
                ? Text(
                    initial,
                    style: context.textTheme.labelSmall?.copyWith(
                      fontSize: 9,
                      color: context.colors.onPrimaryContainer,
                      fontWeight: FontWeight.w600,
                    ),
                  )
                : null,
          ),
          Positioned(
            right: -1,
            bottom: -1,
            child: Container(
              width: 9,
              height: 9,
              decoration: BoxDecoration(
                color: _presenceColor(context, presence),
                shape: BoxShape.circle,
                border: Border.all(
                  color: context.theme.scaffoldBackgroundColor,
                  width: 1.5,
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Color _presenceColor(BuildContext context, String presence) {
    return switch (presence) {
      'online' => context.appColors.success,
      'away' => context.appColors.warning,
      _ => context.colors.outline,
    };
  }
}
