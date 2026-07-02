part of '../compose_bar.dart';

List<ChannelMember> _filterMembers(
  List<ChannelMember> members,
  String? query,
  String? currentPubkey,
  Map<String, UserProfile> userCache,
) {
  if (query == null) return const [];
  final q = query.toLowerCase();
  return members
      .where(
        (m) =>
            currentPubkey == null ||
            m.pubkey.toLowerCase() != currentPubkey.toLowerCase(),
      )
      .where((m) {
        if (q.isEmpty) return true;
        final profile = userCache[m.pubkey.toLowerCase()];
        final name = (profile?.displayName ?? m.displayName ?? '')
            .toLowerCase();
        final firstName = name.split(RegExp(r'\s+')).first;
        return name.startsWith(q) ||
            firstName.startsWith(q) ||
            name.contains(q);
      })
      .take(6)
      .toList();
}

class _MentionSuggestions extends StatelessWidget {
  final List<ChannelMember> suggestions;
  final Map<String, UserProfile> userCache;
  final String? currentPubkey;
  final void Function(ChannelMember) onSelect;

  const _MentionSuggestions({
    required this.suggestions,
    required this.userCache,
    required this.currentPubkey,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      constraints: const BoxConstraints(maxHeight: 240),
      clipBehavior: Clip.hardEdge,
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: const BorderRadius.vertical(
          top: Radius.circular(Radii.dialog),
        ),
        boxShadow: [
          BoxShadow(
            color: context.colors.shadow.withValues(alpha: 0.08),
            blurRadius: 8,
            offset: const Offset(0, -2),
          ),
        ],
      ),
      child: ListView.separated(
        shrinkWrap: true,
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        itemCount: suggestions.length,
        separatorBuilder: (_, _) => const SizedBox.shrink(),
        itemBuilder: (context, index) {
          final member = suggestions[index];
          final profile = userCache[member.pubkey.toLowerCase()];
          final name = profile?.displayName?.trim().isNotEmpty == true
              ? profile!.displayName!.trim()
              : member.labelFor(currentPubkey);
          final avatarUrl = profile?.avatarUrl;
          final initial =
              (profile?.displayName?.trim().isNotEmpty == true
                      ? profile!.displayName!.trim()
                      : member.pubkey)[0]
                  .toUpperCase();

          return ListTile(
            dense: true,
            visualDensity: VisualDensity.compact,
            leading: CircleAvatar(
              radius: 14,
              backgroundColor: context.colors.primaryContainer,
              backgroundImage: avatarUrl != null
                  ? NetworkImage(avatarUrl)
                  : null,
              child: avatarUrl == null
                  ? Text(
                      initial,
                      style: context.textTheme.labelSmall?.copyWith(
                        color: context.colors.onPrimaryContainer,
                      ),
                    )
                  : null,
            ),
            title: Text(name, style: context.textTheme.bodyMedium),
            trailing: member.isBot
                ? Icon(
                    LucideIcons.bot,
                    size: 14,
                    color: context.colors.onSurfaceVariant,
                  )
                : null,
            onTap: () => onSelect(member),
          );
        },
      ),
    );
  }
}

@visibleForTesting
List<Channel> filterChannels(List<Channel> channels, String? query) {
  if (query == null) return const [];
  final q = query.toLowerCase();
  return channels
      .where((c) => c.channelType != 'dm')
      .where((c) {
        if (q.isEmpty) return true;
        return c.name.toLowerCase().contains(q);
      })
      .take(8)
      .toList();
}

class _ChannelSuggestions extends StatelessWidget {
  final List<Channel> suggestions;
  final void Function(Channel) onSelect;

  const _ChannelSuggestions({
    required this.suggestions,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      constraints: const BoxConstraints(maxHeight: 240),
      clipBehavior: Clip.hardEdge,
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: const BorderRadius.vertical(
          top: Radius.circular(Radii.dialog),
        ),
        boxShadow: [
          BoxShadow(
            color: context.colors.shadow.withValues(alpha: 0.08),
            blurRadius: 8,
            offset: const Offset(0, -2),
          ),
        ],
      ),
      child: ListView.separated(
        shrinkWrap: true,
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        itemCount: suggestions.length,
        separatorBuilder: (_, _) => const SizedBox.shrink(),
        itemBuilder: (context, index) {
          final channel = suggestions[index];
          return ListTile(
            dense: true,
            visualDensity: VisualDensity.compact,
            leading: Icon(
              channel.isForum ? LucideIcons.messageSquare : LucideIcons.hash,
              size: 18,
              color: context.colors.onSurfaceVariant,
            ),
            title: Text(
              '#${channel.name}',
              style: context.textTheme.bodyMedium,
            ),
            trailing: Text(
              channel.channelType,
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
            onTap: () => onSelect(channel),
          );
        },
      ),
    );
  }
}
