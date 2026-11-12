part of '../channel_detail_page.dart';

double _scaledTextHeight(BuildContext context, TextStyle style) {
  return MediaQuery.textScalerOf(context).scale(style.fontSize ?? 0) *
      (style.height ?? 1);
}

double _twoLineAppBarTitleContentHeight(BuildContext context) {
  final titleStyle = context.textTheme.titleSmall;
  final subtitleStyle = context.textTheme.bodySmall;
  if (titleStyle == null || subtitleStyle == null) return 40;
  return max(
    40,
    _scaledTextHeight(context, titleStyle) +
        _scaledTextHeight(context, subtitleStyle),
  );
}

class _ConversationAppBarTitle extends StatelessWidget {
  const _ConversationAppBarTitle({
    required this.channel,
    required this.label,
    required this.subtitle,
    required this.presence,
    required this.onTap,
    this.avatars = const [],
  });

  final Channel channel;
  final List<IosNavigationAction> avatars;
  final String label;
  final String subtitle;
  final String? presence;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final prefix = channel.isDm ? 'dm' : 'channel';
    final liftedAnimatedPortrait =
        defaultTargetPlatform == TargetPlatform.iOS &&
        avatars.length == 1 &&
        DmHeaderPortrait.playsAnimation(
          avatars.first.imageUrl,
          avatars.first.avatarIsAgent,
        );
    final compactCapsule =
        defaultTargetPlatform == TargetPlatform.android && avatars.isNotEmpty;
    if (compactCapsule) {
      return Semantics(
        button: true,
        child: Align(
          alignment: Alignment.centerLeft,
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxWidth: conversationTitleMaxWidth(
                MediaQuery.sizeOf(context).width,
              ),
            ),
            child: NavigationButtonSurface(
              capsule: true,
              child: Tooltip(
                message: channel.isDm
                    ? 'Open conversation details'
                    : 'Open channel settings',
                child: InkWell(
                  key: ValueKey('$prefix-header-settings-trigger'),
                  onTap: onTap,
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(minHeight: 48),
                    child: Padding(
                      padding: const EdgeInsets.fromLTRB(6, 4, 14, 4),
                      child: Row(
                        key: ValueKey('$prefix-header-text-stack'),
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          ExcludeSemantics(
                            child: SizedBox.square(
                              key: const ValueKey('dm-header-avatar'),
                              dimension: 36,
                              child: FittedBox(
                                child: DmHeaderCluster(
                                  count: avatars.length,
                                  builder: (i, size) => Transform.scale(
                                    scale: avatars[i].avatarIsAgent ? 0.9 : 1,
                                    child: DmHeaderPortrait(
                                      imageUrl: avatars[i].imageUrl,
                                      size: size,
                                      standalone: avatars.length == 1,
                                      isAgent: avatars[i].avatarIsAgent,
                                      initial: avatars[i].avatarInitial ?? '?',
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                          const SizedBox(width: 8),
                          Flexible(
                            child: Text(
                              label,
                              key: ValueKey('$prefix-header-name'),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: context.textTheme.titleSmall?.copyWith(
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                          ),
                          if (channel.isEphemeral) ...[
                            const SizedBox(width: Grid.quarter),
                            _HeaderEphemeralBadge(channel: channel),
                          ],
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    }
    final title = Semantics(
      button: true,
      child: Tooltip(
        message: channel.isDm
            ? 'Open conversation details'
            : 'Open channel settings',
        child: InkWell(
          key: ValueKey('$prefix-header-settings-trigger'),
          borderRadius: BorderRadius.circular(Radii.md),
          onTap: onTap,
          child: ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 48),
            child: Column(
              key: ValueKey('$prefix-header-text-stack'),
              mainAxisSize: MainAxisSize.min,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    if (defaultTargetPlatform != TargetPlatform.android &&
                        !channel.isDm &&
                        channel.visibility == 'private') ...[
                      Icon(
                        BuzzIcons.lock,
                        size: 14,
                        color: context.colors.onSurfaceVariant,
                        semanticLabel: 'Private channel',
                      ),
                      const SizedBox(width: Grid.quarter),
                    ],
                    Flexible(
                      child: Text(
                        label,
                        key: ValueKey('$prefix-header-name'),
                        textAlign: TextAlign.center,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.titleSmall?.copyWith(
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                    if (channel.isEphemeral) ...[
                      const SizedBox(width: Grid.quarter),
                      _HeaderEphemeralBadge(channel: channel),
                    ],
                  ],
                ),
                if (subtitle.isNotEmpty)
                  Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      if (presence != null) ...[
                        ExcludeSemantics(
                          child: Container(
                            key: const ValueKey('dm-header-presence-dot'),
                            width: 6,
                            height: 6,
                            decoration: BoxDecoration(
                              shape: BoxShape.circle,
                              color: switch (presence) {
                                'online' => context.appColors.success,
                                'away' => context.appColors.warning,
                                _ => context.colors.outline,
                              },
                            ),
                          ),
                        ),
                        const SizedBox(width: Grid.quarter),
                      ],
                      Flexible(
                        child: Text(
                          subtitle,
                          key: ValueKey(
                            channel.isDm
                                ? 'dm-header-presence'
                                : 'channel-header-member-count',
                          ),
                          textAlign: TextAlign.center,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: context.textTheme.bodySmall?.copyWith(
                            color: context.colors.onSurface.withValues(
                              alpha: 0.65,
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
              ],
            ),
          ),
        ),
      ),
    );
    if (avatars.isEmpty) {
      return ConstrainedBox(
        constraints: BoxConstraints(
          maxWidth: conversationTitleMaxWidth(MediaQuery.sizeOf(context).width),
        ),
        child: title,
      );
    }
    return SizedBox(
      width: double.infinity,
      height: 56 + max(48, _twoLineAppBarTitleContentHeight(context)),
      child: Stack(
        alignment: Alignment.topCenter,
        children: [
          Positioned(
            top: liftedAnimatedPortrait ? 42 : 56,
            left: 0,
            right: 0,
            child: Center(
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  maxWidth: conversationTitleMaxWidth(
                    MediaQuery.sizeOf(context).width,
                  ),
                ),
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  excludeFromSemantics: true,
                  onTap: onTap,
                  child: NavigationButtonSurface(
                    capsule: true,
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
                      child: title,
                    ),
                  ),
                ),
              ),
            ),
          ),
          Transform.translate(
            offset: Offset(0, liftedAnimatedPortrait ? -4 : 0),
            child: ExcludeSemantics(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: onTap,
                child: DmHeaderCluster(
                  key: const ValueKey('dm-header-avatar'),
                  count: avatars.length,
                  builder: (i, size) => DmHeaderPortrait(
                    imageUrl: avatars[i].imageUrl,
                    size: size,
                    standalone: avatars.length == 1,
                    isAgent: avatars[i].avatarIsAgent,
                    initial: avatars[i].avatarInitial ?? '?',
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

// Both renderers subscribe to the same conversation identities.
({String label, List<IosNavigationAction> avatars}) _watchDmHeader(
  WidgetRef ref,
  Channel channel,
  String? currentPubkey,
  Set<String> agentPubkeys, {
  List<ChannelMember>? soloMembers,
  bool personalChat = false,
}) {
  final normalizedCurrent = currentPubkey?.toLowerCase();
  final members = !channel.isDm
      ? (soloMembers?.map((member) => member.pubkey) ??
            (personalChat ? [?currentPubkey] : const <String>[]))
      : channel.participantPubkeys;
  final normalized = members.map((key) => key.toLowerCase()).toSet().toList();
  final others = normalized.where((key) => key != normalizedCurrent).toList();
  final portraits =
      (channel.isDm || personalChat
              ? (others.isEmpty ? normalized : others)
              : normalized)
          .take(4)
          .toList();
  if (personalChat && portraits.isEmpty && normalizedCurrent != null) {
    portraits.add(normalizedCurrent);
  }
  final avatars = <IosNavigationAction>[];
  for (final key in portraits) {
    final person = ref.watch(userCacheProvider.select((users) => users[key]));
    if (person == null) ref.read(userCacheProvider.notifier).preload([key]);
    avatars.add(
      IosNavigationAction(
        label: person?.label ?? key,
        avatarIdentity: key,
        avatarInitial:
            person?.initial ?? (key.isEmpty ? '?' : key[0].toUpperCase()),
        imageUrl: person?.avatarUrl,
        avatarIsAgent:
            agentPubkeys.contains(key) ||
            (person?.isAgent ?? false) ||
            (soloMembers?.any(
                  (member) =>
                      member.pubkey.toLowerCase() == key && member.isBot,
                ) ??
                false),
      ),
    );
  }
  if (avatars.isEmpty) {
    avatars.add(
      IosNavigationAction(
        label: channel.name,
        avatarInitial: dmAvatarInitial(channel, currentPubkey: currentPubkey),
      ),
    );
  }
  return (
    avatars: avatars,
    label: !channel.isDm
        ? channel.name
        : ref.watch(
            identityNameSourcesProvider.select(
              (names) => resolveDmChannelDisplayLabel(
                channel,
                currentPubkey: currentPubkey,
                names: names,
              ),
            ),
          ),
  );
}
