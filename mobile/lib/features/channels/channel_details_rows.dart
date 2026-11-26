part of 'channel_actions_sheet.dart';

class _ChannelDetailsHero extends StatelessWidget {
  const _ChannelDetailsHero({
    required this.channel,
    this.icon,
    required this.nameKey,
    required this.displayName,
    this.avatar,
  });

  final Channel channel;
  final IconData? icon;
  final GlobalKey nameKey;
  final String displayName;
  final Widget? avatar;

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
          avatar ??
              Container(
                key: const ValueKey('channel-details-avatar'),
                width: 80,
                height: 80,
                decoration: BoxDecoration(
                  color: context.controlSurface,
                  shape: BoxShape.circle,
                ),
                child: Icon(
                  icon ?? _channelDetailsIcon(channel),
                  size: 32,
                  color: context.colors.onPrimaryContainer,
                ),
              ),
          const SizedBox(height: Grid.xxs),
          KeyedSubtree(
            key: const ValueKey('channel-details-name'),
            child: Text(
              displayName,
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
    this.size = Grid.lg,
    this.padding = Grid.xxs,
    required this.contextualLabel,
    required this.avatarUrl,
    this.isArchived = false,
  });

  final double size;
  final double padding;
  final ChannelMember member;
  final String? currentPubkey;
  final FutureOr<void> Function(BuildContext context, String pubkey)
  onMemberTap;
  final String? displayName;

  /// The channel's contextual label for this member, when resolved.
  final String? contextualLabel;
  final String? avatarUrl;

  /// Known-archived: the row says so, in text a screen reader reads too.
  final bool isArchived;

  @override
  Widget build(BuildContext context) {
    final isSelf = member.pubkey.toLowerCase() == currentPubkey?.toLowerCase();
    final hasName = displayName?.trim().isNotEmpty == true;
    final name = isSelf
        ? 'You'
        : hasName
        ? displayName!.trim()
        : member.labelFor(currentPubkey);
    final label = isSelf ? name : contextualLabel ?? name;
    // Self/named initials come from the visible label; unnamed members stay
    // keyed to the hex public key so the compact-npub label doesn't render
    // `N` for everyone.
    final initial = isSelf || hasName
        ? name[0].toUpperCase()
        : (member.pubkey.isNotEmpty ? member.pubkey[0].toUpperCase() : '?');
    final roleLabel = isArchived
        ? 'Archived \u00b7 ${_channelMemberRoleLabel(member.role)}'
        : _channelMemberRoleLabel(member.role);
    final titleStyle = context.textTheme.bodyLarge;
    final roleStyle = context.textTheme.bodySmall?.copyWith(
      color: context.colors.onSurfaceVariant,
    );

    return AppListRowRaw(
      leading: AvatarImage(
        imageUrl: avatarUrl,
        radius: size / 2,
        backgroundColor: context.controlSurface,
        fallback: Text(initial),
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
      verticalPadding: padding,
    );
  }
}

String _channelMemberRoleLabel(String role) {
  if (role == 'bot') return 'Agent';
  if (role.isEmpty) return 'Member';
  return '${role[0].toUpperCase()}${role.substring(1)}';
}

IconData _channelDetailsIcon(Channel channel) {
  if (channel.isSolo) return BuzzIcons.messageCircle;
  if (channel.isPrivate) return BuzzIcons.lock;
  if (channel.isForum) return BuzzIcons.messageSquareText;
  return BuzzIcons.hash;
}

class _ChannelDetailsChevron extends StatelessWidget {
  const _ChannelDetailsChevron();

  @override
  Widget build(BuildContext context) => Icon(
    BuzzIcons.chevronRight,
    size: 18,
    color: context.colors.onSurfaceVariant,
  );
}
