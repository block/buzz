part of 'channel_actions_sheet.dart';

const _channelMemberPreviewLimit = 5;
const _channelDetailsHeaderFrostScrollDistance = Grid.xxl;
const _channelDetailsSectionPadding = Grid.twelve;

/// Opens the combined mobile channel settings page.
Future<bool?> showChannelDetailsPage({
  required BuildContext context,
  required Channel channel,
  required String? currentPubkey,
  required FutureOr<void> Function(BuildContext context, String pubkey)
  onMemberTap,
  String? sectionId,
  Widget Function(Widget)? soloSettings,
  bool personalChat = false,
}) => Navigator.of(context).push<bool>(
  MaterialPageRoute<bool>(
    builder: (_) => CoveredRouteKeyboardInsets(
      child: ChannelDetailsPage(
        channel: channel,
        currentPubkey: currentPubkey,
        onMemberTap: onMemberTap,
        sectionId: sectionId,
        personalChat: personalChat,
        soloSettings: soloSettings,
      ),
    ),
  ),
);

/// Opens the channel settings page as a bottom sheet, presented like the
/// channel list's long-press actions sheet.
Future<bool?> showChannelDetailsSheet({
  required BuildContext context,
  required Channel channel,
  required String? currentPubkey,
  FutureOr<void> Function(BuildContext context, String pubkey)? onMemberTap,
  String? sectionId,
  bool isUnread = false,

  Widget Function(Widget, Widget Function(MeChatSettings))? soloSettings,
  bool personalChat = false,
}) {
  // Solo reuses the title menu, including its live agent and workspace pickers.
  Widget content(BuildContext context, {bool nativePresentation = false}) =>
      ChannelDetailsPage(
        channel: channel,
        currentPubkey: currentPubkey,
        onMemberTap:
            onMemberTap ??
            (context, pubkey) => showUserProfileSheet(
              context,
              pubkey,
              names: channelIdentityNamesProvider(channel.id),
              contextualActions: (_) =>
                  ChannelMemberProfileActions(channel: channel, pubkey: pubkey),
            ),
        sectionId: sectionId,
        personalChat: personalChat,
        sheetSoloSettings: soloSettings,
        inSheet: true,
        isUnread: isUnread,
        nativePresentation: nativePresentation,
      );
  return _presentChannelListSheet(context, content);
}

/// Combined channel identity, membership, organization, and lifecycle page.
class ChannelDetailsPage extends HookConsumerWidget {
  const ChannelDetailsPage({
    super.key,
    required this.channel,
    required this.currentPubkey,
    required this.onMemberTap,
    this.sectionId,
    this.personalChat = false,
    this.soloSettings,
    this.sheetSoloSettings,
    this.inSheet = false,
    this.isUnread = false,
    this.nativePresentation = false,
  });

  final Channel channel;
  final String? currentPubkey;
  final FutureOr<void> Function(BuildContext context, String pubkey)
  onMemberTap;
  final String? sectionId;

  /// Solo chats are organized into Solo workspaces, not channel sections, so
  /// their details omit the channel-section move.
  final bool personalChat;

  /// Renders only the scrolling content, without the page's app bar, for a
  /// bottom sheet that supplies its own drag handle.
  final bool inSheet;
  final bool isUnread;
  final Widget Function(Widget)? soloSettings;
  final Widget Function(Widget, Widget Function(MeChatSettings))?
  sheetSoloSettings;
  final bool nativePresentation;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final displayedChannel = useState(channel);
    final resolvedChannel = displayedChannel.value;
    final solo = personalChat || resolvedChannel.isSolo;
    final regularChannel = !solo && !resolvedChannel.isDm;
    final noun = solo ? 'chat' : 'channel';
    final lifecycleName = solo
        ? resolvedChannel.name
        : '#${resolvedChannel.name}';
    final aligned = solo || regularChannel;
    final memberSize = aligned ? Grid.md : Grid.lg;
    final memberPadding = aligned ? Grid.xs : Grid.xxs;
    final scrollController = useScrollController();
    final heroNameKey = useMemoized(GlobalKey.new);
    final collapsedTitleOffset = useRef<double?>(null);
    final showCollapsedTitle = useState(false);
    final headerFrostProgress = useState(0.0);
    final isJoining = useState(false);
    final isMuted =
        ref
            .watch(channelMutesProvider)
            .store
            .channels[resolvedChannel.id]
            ?.muted ==
        true;
    final isPinned =
        ref
            .watch(channelPinsProvider)
            .store
            .channels[resolvedChannel.id]
            ?.enabled ==
        true;
    final isStarred =
        ref
            .watch(channelStarsProvider)
            .store
            .channels[resolvedChannel.id]
            ?.starred ==
        true;
    final membersAsync = ref.watch(channelMembersProvider(resolvedChannel.id));
    final members = membersAsync.asData?.value ?? const <ChannelMember>[];
    final agentOwnersAsync = ref.watch(agentOwnersProvider);
    final sectionState = ref.watch(channelSectionsProvider);
    // Desktop's unified workspaces are read-only here.
    final canFile = !personalChat && sectionState.editable;
    final resolvedCurrentPubkey =
        currentPubkey?.toLowerCase() ??
        ref.watch(currentPubkeyProvider)?.toLowerCase();
    final currentMember = members.cast<ChannelMember?>().firstWhere(
      (member) => member?.pubkey.toLowerCase() == resolvedCurrentPubkey,
      orElse: () => null,
    );
    final ownsOwnerAgent =
        resolvedCurrentPubkey != null &&
        members.any(
          (member) =>
              member.isOwner &&
              agentOwnersAsync.value?[member.pubkey.toLowerCase()]
                      ?.toLowerCase() ==
                  resolvedCurrentPubkey,
        );
    final canManageLifecycle =
        currentMember?.isElevated == true || ownsOwnerAgent;
    final canEdit =
        !resolvedChannel.isDm &&
        canManageLifecycle &&
        !resolvedChannel.isArchived;
    final canOpenEditor =
        canEdit || (resolvedChannel.isMember && !resolvedChannel.isArchived);
    final canJoin =
        resolvedChannel.visibility == 'open' &&
        !resolvedChannel.isArchived &&
        !resolvedChannel.isMember &&
        !resolvedChannel.joinsAsGuest;
    final canAddMembers =
        membersAsync.hasValue &&
        !membersAsync.isLoading &&
        !membersAsync.hasError &&
        !resolvedChannel.isArchived &&
        resolvedChannel.canAddMembers(currentMember?.role);
    // The relay's moderation DM only carries its notices; no one joins it.
    final relayUrl = resolvedChannel.isDm
        ? ref.watch(activeCommunityProvider).asData?.value?.relayUrl
        : null;
    final relayAuthor = relayUrl == null
        ? null
        : ref.watch(relayAuthorProvider(relayUrl)).asData?.value;
    final isModerationDm =
        resolvedChannel.isDm &&
        relayAuthor != null &&
        (resolvedChannel.participantPubkeys.any(
              (pubkey) => pubkey.toLowerCase() == relayAuthor,
            ) ||
            members.any(
              (member) => member.pubkey.toLowerCase() == relayAuthor,
            ));
    // People join a DM only through the DM add, which any member can use.
    final canAddToDm =
        resolvedChannel.isDm &&
        currentMember != null &&
        !resolvedChannel.isArchived;
    final canArchive = !resolvedChannel.isArchived && canManageLifecycle;
    final canUnarchive = resolvedChannel.isArchived && canManageLifecycle;
    final canDelete =
        !resolvedChannel.isArchived &&
        (currentMember?.isOwner == true || ownsOwnerAgent);
    final soleOwner =
        currentMember?.isOwner == true &&
        !members.any((member) => member.isOwner && member != currentMember);
    final canLeave =
        resolvedChannel.isMember &&
        !resolvedChannel.isArchived &&
        (solo
            ? currentMember != null &&
                  !currentMember.isOwner &&
                  !ownsOwnerAgent &&
                  agentOwnersAsync.hasValue
            : resolvedChannel.isDm || !soleOwner);
    final lifecycleCapabilitiesLoading =
        membersAsync.isLoading || agentOwnersAsync.isLoading;
    final lifecycleCapabilitiesUnavailable =
        membersAsync.hasError || agentOwnersAsync.hasError;
    final memberCount =
        membersAsync.value?.length ?? resolvedChannel.memberCount;
    final memberLabel =
        '$memberCount ${memberCount == 1 ? 'member' : 'members'}';
    // People, agents, then known-archived people and agents, as in the
    // members sheet.
    final archived = ref.watch(hiddenArchivedIdentitiesProvider);
    bool isArchived(ChannelMember member) =>
        archived.contains(member.pubkey.toLowerCase());
    final previewMembers = [
      for (final archivedPart in [false, true])
        for (final bot in [false, true])
          ...members.where(
            (member) =>
                member.isBot == bot && isArchived(member) == archivedPart,
          ),
    ].take(_channelMemberPreviewLimit).toList();
    final userCache = ref.watch(userCacheProvider);
    final memberLabels = watchChannelIdentityLabels(
      ref,
      resolvedChannel.id,
      previewMembers.map((member) => member.pubkey),
    );
    final participants = resolvedChannel.participantPubkeys
        .map((pubkey) => pubkey.toLowerCase())
        .toSet();
    final isDirectDm = resolvedChannel.isDm && participants.length == 2;
    final counterpart =
        isDirectDm && participants.contains(resolvedCurrentPubkey)
        ? participants.where((pubkey) => pubkey != resolvedCurrentPubkey).single
        : null;
    final counterpartProfile = userCache[counterpart];
    final identityNames = ref.watch(identityNameSourcesProvider);
    final displayName = counterpart != null
        ? dmParticipantNames(
            resolvedChannel,
            identityNames,
          ).labelFor(counterpart)
        : resolveDmChannelDisplayLabel(
            resolvedChannel,
            currentPubkey: resolvedCurrentPubkey,
            names: identityNames,
          );
    final participantKey = participants.join('|');
    useEffect(() {
      if (resolvedChannel.isDm && participants.isNotEmpty) {
        ref.read(userCacheProvider.notifier).preload(participants.toList());
      }
      return null;
    }, [resolvedChannel.isDm, participantKey]);
    final currentSectionId = sectionState.isReady
        ? sectionState.store.assignments[resolvedChannel.id]
        : sectionId;
    final previewPubkeyKey = previewMembers
        .map((member) => member.pubkey.toLowerCase())
        .join('|');

    useEffect(() {
      if (previewMembers.isNotEmpty) {
        ref
            .read(userCacheProvider.notifier)
            .preload(previewMembers.map((member) => member.pubkey).toList());
      }
      return null;
    }, [previewPubkeyKey]);

    useEffect(() {
      void updateCollapsedTitle() {
        final nextFrostProgress = !scrollController.hasClients
            ? 0.0
            : (scrollController.offset /
                      _channelDetailsHeaderFrostScrollDistance)
                  .clamp(0.0, 1.0)
                  .toDouble();
        if ((headerFrostProgress.value - nextFrostProgress).abs() > 0.001) {
          headerFrostProgress.value = nextFrostProgress;
        }

        final threshold = collapsedTitleOffset.value;
        if (threshold == null || !context.mounted) return;
        final nextValue = scrollController.offset >= threshold;
        if (showCollapsedTitle.value != nextValue) {
          showCollapsedTitle.value = nextValue;
        }
      }

      collapsedTitleOffset.value = null;
      scrollController.addListener(updateCollapsedTitle);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final nameContext = heroNameKey.currentContext;
        final renderBox = nameContext?.findRenderObject() as RenderBox?;
        if (renderBox == null || !renderBox.hasSize || !context.mounted) {
          return;
        }
        final nameBottom = renderBox
            .localToGlobal(Offset(0, renderBox.size.height))
            .dy;
        collapsedTitleOffset.value =
            scrollController.offset + nameBottom - frostedAppBarHeight(context);
        updateCollapsedTitle();
      });
      return () => scrollController.removeListener(updateCollapsedTitle);
    }, [scrollController, displayName]);

    Future<void> openMembers() => showBuzzModalBottomSheet<void>(
      context: context,
      title: 'Members',
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => MembersSheet(
        channel: resolvedChannel,
        currentPubkey: resolvedCurrentPubkey,
        onMemberTap: onMemberTap,
      ),
    );

    Future<void> openAddMembers() async {
      await showAddChannelMembersSheet(
        context: context,
        channelId: resolvedChannel.id,
        existingPubkeys: {
          for (final member in members) member.pubkey.toLowerCase(),
        },
        isDm: resolvedChannel.isDm,
        onAddedToDm: (dm) {
          if (!context.mounted) return;
          // This DM grew: show its new members.
          if (dm.id == resolvedChannel.id) {
            displayedChannel.value = dm;
            return;
          }
          // They ended up in another DM: leave these details for that DM.
          Navigator.of(context)
            ..pop()
            ..push(
              MaterialPageRoute<void>(
                builder: (_) => ChannelDetailPage(
                  channel: dm,
                  placement: ChannelRoutePlacement.ordinary,
                ),
              ),
            );
        },
      );
    }

    Future<void> openManageChannel() async {
      final shouldClose = await showManageChannelSheet(
        context: context,
        channel: resolvedChannel,
        canEditDetails: canEdit,
        onChannelUpdated: (updated) => displayedChannel.value = updated,
      );
      if (shouldClose == true && context.mounted) {
        Navigator.of(context).pop(true);
      }
    }

    void toggleStar() {
      final notifier = ref.read(channelStarsProvider.notifier);
      isStarred
          ? notifier.unstarChannel(resolvedChannel.id)
          : notifier.starChannel(resolvedChannel.id);
    }

    void toggleMute() {
      final notifier = ref.read(channelMutesProvider.notifier);
      isMuted
          ? notifier.unmuteChannel(resolvedChannel.id)
          : notifier.muteChannel(resolvedChannel.id);
    }

    Future<void> joinChannel() async {
      if (isJoining.value) return;
      isJoining.value = true;
      try {
        await ref.read(channelActionsProvider).joinChannel(resolvedChannel.id);
        if (context.mounted) Navigator.of(context).pop(false);
      } on ChannelJoinUnconfirmed {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            const SnackBar(
              content: Text(
                'Join sent, but not confirmed yet. Try again to check.',
              ),
            ),
            kind: ToastKind.generic,
          );
        }
      } catch (_) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            const SnackBar(content: Text('Couldn’t join channel. Try again.')),
            kind: ToastKind.error,
          );
        }
      } finally {
        isJoining.value = false;
      }
    }

    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final usesNativeIosGlassBackButton =
        Navigator.canPop(context) &&
        Theme.of(context).platform == TargetPlatform.iOS;
    final renameRow = AppListRow(
      icon: BuzzIcons.pencil,
      title: 'Rename',
      onTap: !canEdit
          ? null
          : () => _renameConversation(
              context,
              ref,
              resolvedChannel,
              onRenamed: (name) =>
                  displayedChannel.value = resolvedChannel.copyWith(name: name),
            ),
    );
    final moveRow = AppListRow(
      icon: BuzzIcons.folderInput,
      title: 'Move to section…',
      trailing: const _ChannelDetailsChevron(),
      onTap: () async {
        await _showMoveSectionSheet(
          context,
          channel: resolvedChannel,
          sectionId: currentSectionId,
        );
      },
    );
    final organizationCard = AppListCard(
      iconColumnWidth: Grid.md,
      verticalPadding: _channelDetailsSectionPadding,
      children: [
        AppListRow(
          key: const ValueKey('channel-details-star-action'),
          icon: isStarred ? BuzzIcons.starOff : BuzzIcons.star,
          title: isStarred ? 'Unstar' : 'Star',
          onTap: toggleStar,
        ),
        if (canFile) moveRow,
      ],
    );
    final channelActionsCard = AppListCard(
      key: const ValueKey('channel-details-channel-card'),
      iconColumnWidth: aligned ? Grid.md : null,
      verticalPadding: _channelDetailsSectionPadding,
      children: [
        AppListRow(
          key: const ValueKey('channel-details-pin-action'),
          icon: isPinned ? BuzzIcons.pinnedOff : BuzzIcons.pinned,
          title: isPinned ? 'Unpin' : 'Pin',
          onTap: () => ref
              .read(channelPinsProvider.notifier)
              .setEnabled(resolvedChannel.id, !isPinned),
        ),
        if (solo)
          AppListRow(
            icon: isStarred ? BuzzIcons.starOff : BuzzIcons.star,
            title: isStarred ? 'Unstar' : 'Star',
            onTap: toggleStar,
          ),
        if (solo && canEdit)
          AppListRow(
            icon: BuzzIcons.pencil,
            title: 'Edit details',
            onTap: openManageChannel,
          ),
        if (!regularChannel && canFile) moveRow,
        AppListRow(
          icon: BuzzIcons.copy,
          title: 'Copy channel name',
          onTap: () {
            copyToClipboard(
              context,
              resolvedChannel.name,
              message: 'Channel name copied',
            );
          },
        ),
        AppListRow(
          icon: BuzzIcons.hash,
          title: 'Copy channel ID',
          onTap: () {
            copyToClipboard(
              context,
              resolvedChannel.id,
              message: 'Channel ID copied',
            );
          },
        ),
      ],
    );
    final leaveRow = AppListRow(
      icon: BuzzIcons.logOut,
      title: 'Leave $noun',
      titleColor: context.colors.error,
      onTap: () => _confirmAndRun(
        context,
        ref,
        title: 'Leave $lifecycleName?',
        body: 'You’ll stop receiving messages from this $noun.',
        confirmLabel: 'Leave',
        action: () =>
            ref.read(channelActionsProvider).leaveChannel(resolvedChannel.id),
      ),
    );
    final membersCard = AppListCard(
      key: const ValueKey('channel-details-members-card'),
      label: solo ? null : memberLabel,
      dividerIndent: Grid.xs + memberSize + Grid.xs,
      iconColumnWidth: aligned ? Grid.md : null,
      verticalPadding: _channelDetailsSectionPadding,
      children: [
        if (canAddMembers)
          AppListRowRaw(
            key: const ValueKey('channel-details-add-members-row'),
            leading: Container(
              width: memberSize,
              height: memberSize,
              decoration: BoxDecoration(
                color: context.colors.surfaceContainer,
                shape: BoxShape.circle,
              ),
              child: Icon(
                BuzzIcons.plus,
                color: context.colors.onSurfaceVariant,
              ),
            ),
            title: Text('Add members', style: context.textTheme.bodyLarge),
            trailing: const _ChannelDetailsChevron(),
            onTap: openAddMembers,
            verticalPadding: memberPadding,
          ),
        if (membersAsync.isLoading && members.isEmpty)
          const AppListRow(
            icon: BuzzIcons.loaderCircle,
            title: 'Loading members…',
            trailing: BuzzLoadingIndicator(
              size: 20,
              semanticLabel: 'Loading members',
            ),
          )
        else if (membersAsync.hasError && members.isEmpty)
          const AppListRow(
            icon: BuzzIcons.triangleAlert,
            title: 'Members unavailable',
          )
        else ...[
          for (final member in previewMembers)
            _ChannelMemberPreviewRow(
              key: ValueKey('channel-details-member-${member.pubkey}'),
              member: member,
              size: memberSize,
              padding: memberPadding,
              currentPubkey: resolvedCurrentPubkey,
              onMemberTap: onMemberTap,
              displayName: userCache[member.pubkey.toLowerCase()]?.displayName,
              contextualLabel: memberLabels[member.pubkey.toLowerCase()],
              avatarUrl: userCache[member.pubkey.toLowerCase()]?.avatarUrl,
              isArchived: isArchived(member),
            ),
          if (memberCount > _channelMemberPreviewLimit)
            AppListRowRaw(
              key: const ValueKey('channel-details-members-row'),
              leading: SizedBox.square(dimension: memberSize),
              title: Text('See all', style: context.textTheme.bodyLarge),
              trailing: const _ChannelDetailsChevron(),
              onTap: openMembers,
              verticalPadding: memberPadding,
            ),
        ],
      ],
    );
    final showLifecycle =
        resolvedChannel.isMember ||
        canJoin ||
        lifecycleCapabilitiesLoading ||
        lifecycleCapabilitiesUnavailable ||
        canArchive ||
        canUnarchive ||
        canDelete;
    final lifecycleCard = AppListCard(
      key: const ValueKey('channel-details-lifecycle-card'),
      iconColumnWidth: aligned ? Grid.md : null,
      verticalPadding: _channelDetailsSectionPadding,
      children: [
        if (resolvedChannel.isSolo && canEdit)
          AppListRow(
            icon: BuzzIcons.hash,
            title: 'Convert to channel',
            onTap: () => _confirmAndRun(
              context,
              ref,
              title: 'Convert to channel?',
              body:
                  'This conversation will appear as a channel. Its messages, members and access settings are kept. Members of the community can discover it if its access is open.',
              confirmLabel: 'Convert',
              action: () => ref
                  .read(channelActionsProvider)
                  .convertSolo(resolvedChannel.id, personalChat: personalChat),
            ),
          ),
        if (canJoin)
          AppListRow(
            icon: BuzzIcons.logIn,
            title: isJoining.value ? 'Joining channel…' : 'Join channel',
            onTap: isJoining.value ? null : joinChannel,
          ),
        if (canLeave && resolvedChannel.isDm && !solo) leaveRow,
        if (lifecycleCapabilitiesLoading)
          const AppListRow(
            icon: BuzzIcons.loaderCircle,
            title: 'Loading channel actions…',
            trailing: BuzzLoadingIndicator(
              size: 20,
              semanticLabel: 'Loading channel actions',
            ),
          )
        else if (lifecycleCapabilitiesUnavailable)
          const AppListRow(
            icon: BuzzIcons.triangleAlert,
            title: 'Channel actions unavailable',
          )
        else ...[
          if (canArchive)
            AppListRow(
              icon: BuzzIcons.archive,
              title: 'Archive $noun',
              onTap: () => _confirmAndRun(
                context,
                ref,
                title: 'Archive $lifecycleName?',
                body: 'The $noun will become read-only.',
                confirmLabel: 'Archive',
                action: () => ref
                    .read(channelActionsProvider)
                    .archiveChannel(resolvedChannel.id),
              ),
            ),
          if (canUnarchive)
            AppListRow(
              icon: BuzzIcons.archiveRestore,
              title: 'Unarchive $noun',
              onTap: () => _confirmAndRun(
                context,
                ref,
                title: 'Unarchive $lifecycleName?',
                body: 'The $noun will become active again.',
                confirmLabel: 'Unarchive',
                action: () => ref
                    .read(channelActionsProvider)
                    .unarchiveChannel(resolvedChannel.id),
              ),
            ),
          if (canDelete)
            AppListRow(
              icon: BuzzIcons.trash2,
              title: 'Delete $noun',
              titleColor: context.colors.error,
              onTap: () => _confirmAndRun(
                context,
                ref,
                title: 'Delete $lifecycleName?',
                body:
                    'This permanently deletes the $noun and cannot be undone.',
                confirmLabel: 'Delete',
                action: () => ref
                    .read(channelActionsProvider)
                    .deleteChannel(resolvedChannel.id),
              ),
            ),
        ],
        if (canLeave && (solo || regularChannel)) leaveRow,
      ],
    );
    final readLabel = isUnread ? 'Mark read' : 'Mark unread';
    void markUnread() {
      if (isUnread) {
        _markChannelReadNow(ref, resolvedChannel.id);
        Navigator.of(context).pop();
        return;
      }
      if (ref
          .read(readStateProvider.notifier)
          .markContextUnread(
            resolvedChannel.id,
            channelId: resolvedChannel.id,
          )) {
        Navigator.of(context).pop(true);
      } else {
        ScaffoldMessenger.of(context).showFeedbackSnackBar(
          const SnackBar(content: Text('Couldn’t mark as unread. Try again.')),
          kind: ToastKind.error,
        );
      }
    }

    final showHuddle =
        (solo || (regularChannel && !resolvedChannel.isForum)) &&
        resolvedChannel.isMember &&
        !resolvedChannel.isArchived &&
        currentMember?.role != 'guest';
    if (nativePresentation) {
      Widget present(MeChatSettings? settings) => _NativeChannelActions(
        title: displayName,
        detailsChannel: resolvedChannel,
        solo: solo,
        heroAvatarUrl: counterpartProfile?.avatarUrl,
        heroIsAgent: counterpartProfile?.isAgent ?? false,
        memberLabel: solo || isDirectDm ? null : memberLabel,

        settings: settings,
        groups: [
          SheetActionSection(
            children: [
              if (solo || regularChannel)
                AppListRow(
                  icon: BuzzIcons.mail,
                  title: readLabel,
                  onTap: markUnread,
                )
              else
                AppListRow(
                  icon: BuzzIcons.star,
                  title: isStarred ? 'Unstar' : 'Star',
                  onTap: toggleStar,
                ),
              AppListRow(
                icon: isMuted ? BuzzIcons.bell : BuzzIcons.bellOff,
                title: isMuted ? 'Unmute' : 'Mute',
                onTap: toggleMute,
              ),
              if (regularChannel)
                AppListRow(
                  icon: BuzzIcons.pencil,
                  title: 'Edit',
                  onTap: canOpenEditor ? openManageChannel : null,
                ),
            ],
          ),
          if (settings != null)
            SheetActionSection(
              children: [
                AppListRow(
                  title: 'Agent',
                  subtitle: settings.agentName,
                  onTap: settings.onAgentTap,
                ),
              ],
            ),
          if (regularChannel) ...[
            SheetActionSection(children: membersCard.children),
            SheetActionSection(children: organizationCard.children),
          ] else if (solo)
            SheetActionSection(
              children: [
                renameRow,
                if (settings != null)
                  AppListRow(
                    icon: BuzzIcons.folder,
                    title: settings.workspaceName,
                    onTap: settings.onWorkspaceTap,
                  ),
              ],
            ),
          if (resolvedChannel.isDm && !solo && !isDirectDm)
            SheetActionSection(children: membersCard.children),
          SheetActionSection(children: channelActionsCard.children),
          if (showHuddle)
            SheetActionSection(
              children: [channelHuddleActionRow(context, ref, resolvedChannel)],
            ),
          if (solo) SheetActionSection(children: membersCard.children),
          if (showLifecycle)
            SheetActionSection(children: lifecycleCard.children),
        ],
      );
      return sheetSoloSettings?.call(renameRow, present) ?? present(null);
    }
    final content = ListView(
      key: const ValueKey('channel-details-page-list'),
      controller: scrollController,
      shrinkWrap: inSheet,
      padding: EdgeInsets.only(
        top: inSheet ? 0 : frostedAppBarHeight(context) + Grid.xs,
        bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.xs,
      ),
      children: [
        _ChannelDetailsHero(
          channel: resolvedChannel,
          icon: solo
              ? BuzzIcons.messageCircle
              : _channelDetailsIcon(resolvedChannel),
          nameKey: heroNameKey,
          displayName: displayName,
          avatar: counterpart == null
              ? null
              : AvatarImage(
                  key: const ValueKey('dm-details-avatar'),
                  imageUrl: counterpartProfile?.avatarUrl,
                  radius: 40,
                  isAgent: counterpartProfile?.isAgent ?? false,
                  backgroundColor: context.controlSurface,
                  fallback: Text(
                    counterpartProfile?.initial ??
                        dmAvatarInitial(
                          resolvedChannel,
                          currentPubkey: resolvedCurrentPubkey,
                        ),
                    style: context.textTheme.headlineMedium?.copyWith(
                      color: context.colors.onPrimaryContainer,
                    ),
                  ),
                ),
        ),
        Padding(
          key: const ValueKey('channel-details-actions'),
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            _channelDetailsSectionPadding,
            Grid.gutter,
            _channelDetailsSectionPadding,
          ),
          child: IntrinsicHeight(
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Expanded(
                  child: solo || regularChannel
                      ? BuzzActionTile(
                          icon: BuzzIcons.mail,
                          key: const ValueKey('channel-details-read-action'),
                          label: readLabel,
                          onTap: markUnread,
                        )
                      : BuzzActionTile(
                          key: const ValueKey('channel-details-star-action'),
                          icon: null,
                          iconWidget: TablerStarIcon(
                            filled: isStarred,
                            color: isStarred
                                ? context.colors.primary
                                : context.colors.onSurface,
                          ),
                          label: isStarred ? 'Unstar' : 'Star',
                          onTap: toggleStar,
                        ),
                ),
                const SizedBox(width: Grid.xxs),
                Expanded(
                  child: BuzzActionTile(
                    key: const ValueKey('channel-details-mute-action'),
                    icon: isMuted ? BuzzIcons.bell : BuzzIcons.bellOff,
                    iconColor: isMuted ? context.colors.primary : null,
                    label: isMuted ? 'Unmute' : 'Mute',
                    onTap: toggleMute,
                  ),
                ),
                if (regularChannel) ...[
                  const SizedBox(width: Grid.xxs),
                  Expanded(
                    child: BuzzActionTile(
                      key: const ValueKey('channel-details-edit-action'),
                      icon: BuzzIcons.pencil,
                      label: 'Edit',
                      isEnabled: canOpenEditor,
                      onTap: openManageChannel,
                    ),
                  ),
                ],
                if (resolvedChannel.isDm && !solo && !isModerationDm) ...[
                  const SizedBox(width: Grid.xxs),
                  Expanded(
                    child: BuzzActionTile(
                      key: const ValueKey('channel-details-add-action'),
                      icon: BuzzIcons.userPlus,
                      label: 'Add',
                      isEnabled: canAddToDm,
                      onTap: openAddMembers,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
        if (solo)
          (sheetSoloSettings?.call(renameRow, (settings) => settings) ??
              soloSettings?.call(renameRow) ??
              AppListCard(
                iconColumnWidth: Grid.md,
                verticalPadding: _channelDetailsSectionPadding,
                children: [renameRow],
              )),
        if (regularChannel) ...[membersCard, organizationCard],
        if (solo || regularChannel) channelActionsCard,
        if (showHuddle)
          AppListCard(
            iconColumnWidth: Grid.md,
            verticalPadding: _channelDetailsSectionPadding,
            children: [ChannelHuddleRow(channel: resolvedChannel)],
          ),
        if (!regularChannel && !isDirectDm) membersCard,
        if (!solo && !regularChannel) channelActionsCard,
        if (showLifecycle) lifecycleCard,
      ],
    );
    if (inSheet) return content;

    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
      appBar: FrostedAppBar(
        nativeTitle: showCollapsedTitle.value ? displayName : '',
        centerTitle: true,
        leading: usesNativeIosGlassBackButton
            ? IosGlassNavigationButton(
                key: const ValueKey('channel-details-ios-glass-back'),
                icon: IosGlassNavigationIcon.back,
                semanticLabel: 'Back',
                onPressed: () => Navigator.of(context).maybePop(),
                width: iosGlassChannelHeaderLeadingWidth,
                buttonCenterX: iosGlassChannelHeaderButtonCenterX,
              )
            : null,
        iconColor: context.colors.primary,
        actions: [
          SizedBox(
            width: usesNativeIosGlassBackButton
                ? iosGlassChannelHeaderLeadingWidth
                : 48,
            height: 48,
          ),
        ],
        frosted: headerFrostProgress.value > 0,

        title: AnimatedSwitcher(
          duration: reducedMotion
              ? Duration.zero
              : const Duration(milliseconds: 160),
          switchInCurve: Curves.easeOutCubic,
          switchOutCurve: Curves.easeOutCubic,
          child: showCollapsedTitle.value
              ? Text(
                  displayName,
                  key: const ValueKey('channel-details-collapsed-title'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                )
              : const SizedBox(
                  key: ValueKey('channel-details-expanded-title-space'),
                ),
        ),
      ),
      body: content,
    );
  }
}
