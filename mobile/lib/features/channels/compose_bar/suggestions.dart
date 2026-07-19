part of '../compose_bar.dart';

class _MentionSuggestions extends StatelessWidget {
  final List<MentionCandidate> suggestions;
  final Map<String, UserProfile> userCache;
  final String? currentPubkey;
  final bool isDmChannel;
  final void Function(MentionCandidate) onSelect;

  const _MentionSuggestions({
    required this.suggestions,
    required this.userCache,
    required this.currentPubkey,
    required this.isDmChannel,
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
          final candidate = suggestions[index];
          final name = candidate.label;
          final avatarUrl =
              candidate.avatarUrl ?? userCache[candidate.pubkey]?.avatarUrl;

          return ListTile(
            dense: true,
            visualDensity: VisualDensity.compact,
            leading: AvatarImage(
              imageUrl: avatarUrl,
              radius: 14,
              backgroundColor: context.colors.primaryContainer,
              fallback: Text(
                name[0].toUpperCase(),
                style: context.textTheme.labelSmall?.copyWith(
                  color: context.colors.onPrimaryContainer,
                ),
              ),
            ),
            title: Text(name, style: context.textTheme.bodyMedium),
            subtitle: _MentionSuggestionInfo.build(
              context,
              candidate: candidate,
              currentPubkey: currentPubkey,
              isDmChannel: isDmChannel,
              userCache: userCache,
            ),
            onTap: () => onSelect(candidate),
          );
        },
      ),
    );
  }
}

/// The secondary info line under a mention suggestion — mirrors desktop's
/// `MentionAutocomplete` subtitle: bot icon + "agent" (or an "admin" badge
/// for human admins), then "managed by …" / "not in channel".
abstract final class _MentionSuggestionInfo {
  static Widget? build(
    BuildContext context, {
    required MentionCandidate candidate,
    required String? currentPubkey,
    required bool isDmChannel,
    required Map<String, UserProfile> userCache,
  }) {
    final ownerLabel = candidate.isAgent
        ? formatOwnerLabel(candidate.ownerPubkey, currentPubkey, userCache)
        : null;
    final notInChannel = !isDmChannel && !candidate.isMember;
    final isAdmin = !candidate.isAgent && candidate.role == 'admin';

    final String? detail;
    if (ownerLabel != null && notInChannel) {
      detail = 'managed by $ownerLabel \u00b7 not in channel';
    } else if (ownerLabel != null) {
      detail = 'managed by $ownerLabel';
    } else if (notInChannel) {
      detail = 'not in channel';
    } else {
      detail = null;
    }

    if (!candidate.isAgent && !isAdmin && detail == null) return null;

    final style = context.textTheme.labelSmall?.copyWith(
      color: context.colors.onSurfaceVariant,
    );

    return Row(
      children: [
        if (candidate.isAgent) ...[
          Icon(
            LucideIcons.bot,
            size: 12,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(width: Grid.half),
          Text('agent', style: style),
        ] else if (isAdmin)
          Container(
            padding: const EdgeInsets.symmetric(
              horizontal: Grid.xxs,
              vertical: 1,
            ),
            decoration: BoxDecoration(
              color: context.colors.secondaryContainer,
              borderRadius: BorderRadius.circular(Radii.sm),
            ),
            child: Text(
              'admin',
              style: style?.copyWith(
                color: context.colors.onSecondaryContainer,
              ),
            ),
          ),
        if (detail != null) ...[
          if (candidate.isAgent || isAdmin) const SizedBox(width: Grid.xxs),
          Flexible(
            child: Text(detail, style: style, overflow: TextOverflow.ellipsis),
          ),
        ],
      ],
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
