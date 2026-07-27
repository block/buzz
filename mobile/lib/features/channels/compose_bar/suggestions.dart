part of '../compose_bar.dart';

class _SuggestionPanelMotion extends StatelessWidget {
  final Duration duration;
  final Widget child;

  const _SuggestionPanelMotion({required this.duration, required this.child});

  @override
  Widget build(BuildContext context) {
    return AnimatedSwitcher(
      duration: duration,
      reverseDuration: duration,
      layoutBuilder: (currentChild, previousChildren) => Stack(
        alignment: Alignment.bottomLeft,
        clipBehavior: Clip.none,
        children: [...previousChildren, ?currentChild],
      ),
      transitionBuilder: (child, animation) {
        final curvedAnimation = CurvedAnimation(
          parent: animation,
          curve: Curves.easeOutCubic,
          reverseCurve: Curves.easeInCubic,
        );

        return AnimatedBuilder(
          animation: curvedAnimation,
          child: child,
          builder: (context, child) => IgnorePointer(
            ignoring: animation.status == AnimationStatus.reverse,
            child: Opacity(
              opacity: curvedAnimation.value,
              child: Transform.translate(
                offset: Offset(0, Grid.xs * (1 - curvedAnimation.value)),
                child: child,
              ),
            ),
          ),
        );
      },
      child: child,
    );
  }
}

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
        borderRadius: BorderRadius.circular(Radii.dialog),
        border: Border.all(
          color: Colors.black.withValues(alpha: 0.04),
          width: 1,
        ),
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
              radius: 18,
              backgroundColor: context.colors.primaryContainer,
              fallback: Text(
                name[0].toUpperCase(),
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onPrimaryContainer,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ),
            title: Text(name, style: context.textTheme.titleSmall),
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
        borderRadius: BorderRadius.circular(Radii.dialog),
        border: Border.all(
          color: Colors.black.withValues(alpha: 0.04),
          width: 1,
        ),
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
            horizontalTitleGap: 0,
            leading: SizedBox.square(
              dimension: 36,
              child: Icon(
                LucideIcons.hash,
                size: 20,
                color: context.colors.onSurfaceVariant,
              ),
            ),
            title: Text(channel.name, style: context.textTheme.bodyLarge),
            onTap: () => onSelect(channel),
          );
        },
      ),
    );
  }
}
