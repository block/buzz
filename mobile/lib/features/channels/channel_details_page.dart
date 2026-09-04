part of 'channel_actions_sheet.dart';

const _channelMemberPreviewLimit = 5;
const _channelDetailsHeaderFrostScrollDistance = Grid.xxl;
const _channelDetailsHeaderFrostMaxBlurSigma = 20.0;
const _channelDetailsSectionPadding = Grid.twelve;

/// Opens the combined mobile channel settings page.
Future<bool?> showChannelDetailsPage({
  required BuildContext context,
  required Channel channel,
  required String? currentPubkey,
  required void Function(BuildContext context, String pubkey) onMemberTap,
  String? sectionId,
}) => Navigator.of(context).push<bool>(
  MaterialPageRoute<bool>(
    builder: (_) => ChannelDetailsPage(
      channel: channel,
      currentPubkey: currentPubkey,
      onMemberTap: onMemberTap,
      sectionId: sectionId,
    ),
  ),
);

/// Combined channel identity, membership, organization, and lifecycle page.
class ChannelDetailsPage extends HookConsumerWidget {
  const ChannelDetailsPage({
    super.key,
    required this.channel,
    required this.currentPubkey,
    required this.onMemberTap,
    this.sectionId,
  });

  final Channel channel;
  final String? currentPubkey;
  final void Function(BuildContext context, String pubkey) onMemberTap;
  final String? sectionId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final displayedChannel = useState(channel);
    final resolvedChannel = displayedChannel.value;
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
    final canEdit = canManageLifecycle && !resolvedChannel.isArchived;
    final canJoin =
        resolvedChannel.visibility == 'open' &&
        !resolvedChannel.isArchived &&
        !resolvedChannel.isMember;
    final canAddMembers =
        membersAsync.hasValue &&
        !membersAsync.isLoading &&
        !membersAsync.hasError &&
        !resolvedChannel.isArchived &&
        resolvedChannel.canAddMembers(currentMember?.role);
    final canArchive = !resolvedChannel.isArchived && canManageLifecycle;
    final canUnarchive = resolvedChannel.isArchived && canManageLifecycle;
    final canDelete =
        !resolvedChannel.isArchived &&
        (currentMember?.isOwner == true || ownsOwnerAgent);
    final lifecycleCapabilitiesLoading =
        membersAsync.isLoading || agentOwnersAsync.isLoading;
    final lifecycleCapabilitiesUnavailable =
        membersAsync.hasError || agentOwnersAsync.hasError;
    final memberCount =
        membersAsync.value?.length ?? resolvedChannel.memberCount;
    final memberLabel =
        '$memberCount ${memberCount == 1 ? 'member' : 'members'}';
    final previewMembers = members.take(_channelMemberPreviewLimit).toList();
    final userCache = ref.watch(userCacheProvider);
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
    }, [scrollController, resolvedChannel.name]);

    Future<void> openMembers() => showBuzzModalBottomSheet<void>(
      context: context,
      title: 'Members',
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => MembersSheet(
        channel: resolvedChannel,
        currentPubkey: resolvedCurrentPubkey,
      ),
    );

    Future<void> openAddMembers() async {
      final mediaQuery = MediaQuery.of(context);
      await showBuzzModalBottomSheet<bool>(
        context: context,
        title: 'Add members',
        isScrollControlled: true,
        showDragHandle: true,
        constraints: BoxConstraints(
          maxWidth: 640,
          maxHeight:
              mediaQuery.size.height - mediaQuery.viewPadding.top - Grid.xs,
        ),
        builder: (_) => AddChannelMembersSheet(
          channelId: resolvedChannel.id,
          existingPubkeys: {
            for (final member in members) member.pubkey.toLowerCase(),
          },
        ),
      );
    }

    Future<void> openManageChannel() async {
      final shouldClose = await showBuzzModalBottomSheet<bool>(
        context: context,
        title: 'Manage channel',
        isScrollControlled: true,
        showDragHandle: true,
        constraints: BoxConstraints(
          maxWidth: 640,
          maxHeight: MediaQuery.sizeOf(context).height * 0.9,
        ),
        builder: (_) => ManageChannelSheet(
          channel: resolvedChannel,
          canEditDetails: canEdit,
          onChannelUpdated: (updated) => displayedChannel.value = updated,
        ),
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
      } catch (_) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Couldn’t join channel. Try again.')),
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
    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
      appBar: FrostedAppBar(
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
        frostedSurfaceOpacity: 0.5 * headerFrostProgress.value,
        frostedBlurSigma:
            _channelDetailsHeaderFrostMaxBlurSigma * headerFrostProgress.value,
        showBottomDivider: headerFrostProgress.value > 0,
        bottomDividerOpacity: 0.07 * headerFrostProgress.value,
        title: AnimatedSwitcher(
          duration: reducedMotion
              ? Duration.zero
              : const Duration(milliseconds: 160),
          switchInCurve: Curves.easeOutCubic,
          switchOutCurve: Curves.easeOutCubic,
          child: showCollapsedTitle.value
              ? Text(
                  resolvedChannel.name,
                  key: const ValueKey('channel-details-collapsed-title'),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                )
              : const SizedBox(
                  key: ValueKey('channel-details-expanded-title-space'),
                ),
        ),
      ),
      body: ListView(
        key: const ValueKey('channel-details-page-list'),
        controller: scrollController,
        padding: EdgeInsets.only(
          top: frostedAppBarHeight(context) + Grid.xs,
          bottom: MediaQuery.viewPaddingOf(context).bottom + Grid.xs,
        ),
        children: [
          _ChannelDetailsHero(channel: resolvedChannel, nameKey: heroNameKey),
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
                    child: BuzzActionTile(
                      key: const ValueKey('channel-details-star-action'),
                      icon: null,
                      iconWidget: LucideStarIcon(
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
                      icon: isMuted ? LucideIcons.bell : LucideIcons.bellOff,
                      iconColor: isMuted ? context.colors.primary : null,
                      label: isMuted ? 'Unmute' : 'Mute',
                      onTap: toggleMute,
                    ),
                  ),
                  const SizedBox(width: Grid.xxs),
                  Expanded(
                    child: BuzzActionTile(
                      key: const ValueKey('channel-details-edit-action'),
                      icon: LucideIcons.pencil,
                      label: 'Edit',
                      isEnabled: canEdit,
                      onTap: openManageChannel,
                    ),
                  ),
                ],
              ),
            ),
          ),
          AppListCard(
            key: const ValueKey('channel-details-members-card'),
            label: memberLabel,
            dividerIndent: Grid.xs + 40 + Grid.xs,
            verticalPadding: _channelDetailsSectionPadding,
            children: [
              if (canAddMembers)
                AppListRowRaw(
                  key: const ValueKey('channel-details-add-members-row'),
                  leading: Container(
                    width: 40,
                    height: 40,
                    decoration: BoxDecoration(
                      color: context.colors.surfaceContainer,
                      shape: BoxShape.circle,
                    ),
                    child: Icon(
                      LucideIcons.plus,
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                  title: Text(
                    'Add members',
                    style: context.textTheme.bodyLarge,
                  ),
                  trailing: const _ChannelDetailsChevron(),
                  onTap: openAddMembers,
                  verticalPadding: Grid.xxs,
                ),
              if (membersAsync.isLoading && members.isEmpty)
                const AppListRow(
                  icon: LucideIcons.loaderCircle,
                  title: 'Loading members…',
                  trailing: BuzzLoadingIndicator(
                    size: 20,
                    semanticLabel: 'Loading members',
                  ),
                )
              else if (membersAsync.hasError && members.isEmpty)
                const AppListRow(
                  icon: LucideIcons.triangleAlert,
                  title: 'Members unavailable',
                )
              else ...[
                for (final member in previewMembers)
                  _ChannelMemberPreviewRow(
                    key: ValueKey('channel-details-member-${member.pubkey}'),
                    member: member,
                    currentPubkey: resolvedCurrentPubkey,
                    onMemberTap: onMemberTap,
                    displayName:
                        userCache[member.pubkey.toLowerCase()]?.displayName,
                    avatarUrl:
                        userCache[member.pubkey.toLowerCase()]?.avatarUrl,
                  ),
                AppListRowRaw(
                  key: const ValueKey('channel-details-members-row'),
                  leading: const SizedBox.square(dimension: 40),
                  title: Text('See all', style: context.textTheme.bodyLarge),
                  trailing: const _ChannelDetailsChevron(),
                  onTap: openMembers,
                  verticalPadding: Grid.xxs,
                ),
              ],
            ],
          ),
          AppListCard(
            key: const ValueKey('channel-details-channel-card'),
            verticalPadding: _channelDetailsSectionPadding,
            children: [
              AppListRow(
                icon: LucideIcons.folderInput,
                title: 'Move to section…',
                trailing: const _ChannelDetailsChevron(),
                onTap: () async {
                  await _showMoveSectionSheet(
                    context,
                    ref,
                    channel: resolvedChannel,
                    sectionId: currentSectionId,
                  );
                },
              ),
              AppListRow(
                icon: LucideIcons.copy,
                title: 'Copy channel name',
                onTap: () {
                  copyToClipboard(
                    context,
                    resolvedChannel.name,
                    message: 'Channel name copied to clipboard',
                  );
                },
              ),
              AppListRow(
                icon: LucideIcons.hash,
                title: 'Copy channel ID',
                onTap: () {
                  copyToClipboard(
                    context,
                    resolvedChannel.id,
                    message: 'Channel ID copied to clipboard',
                  );
                },
              ),
            ],
          ),
          if (resolvedChannel.isMember ||
              canJoin ||
              lifecycleCapabilitiesLoading ||
              lifecycleCapabilitiesUnavailable ||
              canArchive ||
              canUnarchive ||
              canDelete)
            AppListCard(
              key: const ValueKey('channel-details-lifecycle-card'),
              verticalPadding: _channelDetailsSectionPadding,
              children: [
                if (canJoin)
                  AppListRow(
                    icon: LucideIcons.logIn,
                    title: isJoining.value
                        ? 'Joining channel…'
                        : 'Join channel',
                    onTap: isJoining.value ? null : joinChannel,
                  ),
                if (resolvedChannel.isMember && !resolvedChannel.isArchived)
                  AppListRow(
                    icon: LucideIcons.logOut,
                    title: 'Leave channel',
                    titleColor: context.colors.error,
                    onTap: () => _confirmAndRun(
                      context,
                      ref,
                      title: 'Leave #${resolvedChannel.name}?',
                      body: 'You’ll stop receiving messages from this channel.',
                      confirmLabel: 'Leave',
                      action: () => ref
                          .read(channelActionsProvider)
                          .leaveChannel(resolvedChannel.id),
                    ),
                  ),
                if (lifecycleCapabilitiesLoading)
                  const AppListRow(
                    icon: LucideIcons.loaderCircle,
                    title: 'Loading channel actions…',
                    trailing: BuzzLoadingIndicator(
                      size: 20,
                      semanticLabel: 'Loading channel actions',
                    ),
                  )
                else if (lifecycleCapabilitiesUnavailable)
                  const AppListRow(
                    icon: LucideIcons.triangleAlert,
                    title: 'Channel actions unavailable',
                  )
                else ...[
                  if (canArchive)
                    AppListRow(
                      icon: LucideIcons.archive,
                      title: 'Archive channel',
                      onTap: () => _confirmAndRun(
                        context,
                        ref,
                        title: 'Archive #${resolvedChannel.name}?',
                        body: 'The channel will become read-only.',
                        confirmLabel: 'Archive',
                        action: () => ref
                            .read(channelActionsProvider)
                            .archiveChannel(resolvedChannel.id),
                      ),
                    ),
                  if (canUnarchive)
                    AppListRow(
                      icon: LucideIcons.archiveRestore,
                      title: 'Unarchive channel',
                      onTap: () => _confirmAndRun(
                        context,
                        ref,
                        title: 'Unarchive #${resolvedChannel.name}?',
                        body: 'The channel will become active again.',
                        confirmLabel: 'Unarchive',
                        action: () => ref
                            .read(channelActionsProvider)
                            .unarchiveChannel(resolvedChannel.id),
                      ),
                    ),
                  if (canDelete)
                    AppListRow(
                      icon: LucideIcons.trash2,
                      title: 'Delete channel',
                      titleColor: context.colors.error,
                      onTap: () => _confirmAndRun(
                        context,
                        ref,
                        title: 'Delete #${resolvedChannel.name}?',
                        body:
                            'This permanently deletes the channel and cannot be undone.',
                        confirmLabel: 'Delete',
                        action: () => ref
                            .read(channelActionsProvider)
                            .deleteChannel(resolvedChannel.id),
                      ),
                    ),
                ],
              ],
            ),
        ],
      ),
    );
  }
}

class _ChannelDetailsHero extends StatelessWidget {
  const _ChannelDetailsHero({required this.channel, required this.nameKey});

  final Channel channel;
  final GlobalKey nameKey;

  @override
  Widget build(BuildContext context) {
    final description = channel.description.trim();
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xs,
        Grid.gutter,
        _channelDetailsSectionPadding,
      ),
      child: Column(
        children: [
          Container(
            key: const ValueKey('channel-details-avatar'),
            width: 80,
            height: 80,
            decoration: BoxDecoration(
              color: context.colors.primaryContainer,
              shape: BoxShape.circle,
            ),
            child: Icon(
              _channelDetailsIcon(channel),
              size: 32,
              color: context.colors.onPrimaryContainer,
            ),
          ),
          const SizedBox(height: Grid.xxs),
          KeyedSubtree(
            key: const ValueKey('channel-details-name'),
            child: Text(
              channel.name,
              key: nameKey,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
              style: context.textTheme.headlineSmall?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
          ),
          if (description.isNotEmpty) ...[
            const SizedBox(height: Grid.half),
            Text(
              description,
              key: const ValueKey('channel-details-description'),
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              textAlign: TextAlign.center,
              style: context.textTheme.bodyMedium?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _ChannelMemberPreviewRow extends StatelessWidget {
  const _ChannelMemberPreviewRow({
    super.key,
    required this.member,
    required this.currentPubkey,
    required this.onMemberTap,
    required this.displayName,
    required this.avatarUrl,
  });

  final ChannelMember member;
  final String? currentPubkey;
  final void Function(BuildContext context, String pubkey) onMemberTap;
  final String? displayName;
  final String? avatarUrl;

  @override
  Widget build(BuildContext context) {
    final isSelf = member.pubkey.toLowerCase() == currentPubkey?.toLowerCase();
    final label = isSelf
        ? 'You'
        : displayName?.trim().isNotEmpty == true
        ? displayName!.trim()
        : member.labelFor(currentPubkey);
    final roleLabel = _channelMemberRoleLabel(member.role);
    final titleStyle = context.textTheme.bodyLarge;
    final roleStyle = context.textTheme.bodySmall?.copyWith(
      color: context.colors.onSurfaceVariant,
    );

    return AppListRowRaw(
      leading: AvatarImage(
        imageUrl: avatarUrl,
        radius: 20,
        backgroundColor: context.colors.primaryContainer,
        fallback: Text(label.isEmpty ? '?' : label[0].toUpperCase()),
        isAgent: member.isBot,
      ),
      title: Text.rich(
        TextSpan(
          style: titleStyle,
          children: [
            TextSpan(text: label),
            TextSpan(text: ' · $roleLabel', style: roleStyle),
          ],
        ),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
      trailing: const _ChannelDetailsChevron(),
      onTap: () => onMemberTap(context, member.pubkey),
      verticalPadding: Grid.xxs,
    );
  }
}

String _channelMemberRoleLabel(String role) {
  if (role == 'bot') return 'Agent';
  if (role.isEmpty) return 'Member';
  return '${role[0].toUpperCase()}${role.substring(1)}';
}

IconData _channelDetailsIcon(Channel channel) {
  if (channel.isPrivate) return LucideIcons.lock;
  if (channel.isForum) return LucideIcons.messageSquareText;
  return LucideIcons.hash;
}

class _ChannelDetailsChevron extends StatelessWidget {
  const _ChannelDetailsChevron();

  @override
  Widget build(BuildContext context) => Icon(
    LucideIcons.chevronRight,
    size: 18,
    color: context.colors.onSurfaceVariant,
  );
}
