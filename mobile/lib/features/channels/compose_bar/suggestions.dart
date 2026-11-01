part of '../compose_bar.dart';

/// A mention row with its info line: a dense, compact two-line `ListTile`.
const _mentionRowHeight = 59.0;

/// The chooser shows at most three rows, then scrolls.
const _mentionChooserHeight = _mentionRowHeight * 3 + Grid.xxs * 2;

class _MentionSuggestions extends StatelessWidget {
  final List<MentionCandidate> suggestions;

  /// Shown rows that can no longer be chosen. They stay in place, disabled.
  final Set<String> unavailable;

  /// The directory search failed: show its error and a retry below the rows.
  final bool searchFailed;

  /// Hold the full height, rows at the top, so rows that join late fill
  /// empty space instead of pushing the shown rows up.
  final bool reserveHeight;

  /// The directory search is still running: show a thin progress bar on the
  /// top edge. It overlays the rows, so it never changes the height.
  final bool searching;
  final VoidCallback? onRetry;
  final Map<String, UserProfile> userCache;
  final String? currentPubkey;
  final bool isDmChannel;
  final void Function(MentionCandidate) onSelect;

  const _MentionSuggestions({
    required this.suggestions,
    this.unavailable = const {},
    this.searchFailed = false,
    this.reserveHeight = false,
    this.searching = false,
    this.onRetry,
    required this.userCache,
    required this.currentPubkey,
    required this.isDmChannel,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    return Material(
      key: const ValueKey('mention-suggestions-popover'),
      type: MaterialType.card,
      color: appPopoverColor(context),
      surfaceTintColor: Colors.transparent,
      elevation: appPopoverElevation,
      shadowColor: appPopoverShadowColor(context),
      shape: appPopoverShape(context),
      clipBehavior: Clip.hardEdge,
      child: Stack(
        children: [
          _rows(context),
          if (searching)
            const Positioned(
              top: 0,
              left: 0,
              right: 0,
              child: LinearProgressIndicator(
                key: ValueKey('mention-search-progress'),
                minHeight: 2,
                backgroundColor: Colors.transparent,
                semanticsLabel: 'Searching people',
              ),
            ),
        ],
      ),
    );
  }

  Widget _rows(BuildContext context) {
    return ConstrainedBox(
      constraints: BoxConstraints(
        minHeight: reserveHeight ? _mentionChooserHeight : 0,
        maxHeight: _mentionChooserHeight,
      ),
      child: ListView.separated(
        shrinkWrap: true,
        padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
        itemCount: suggestions.length + (searchFailed ? 1 : 0),
        separatorBuilder: (_, _) => const SizedBox.shrink(),
        itemBuilder: (context, index) {
          if (index == suggestions.length) {
            return ListTile(
              key: const ValueKey('mention-search-error'),
              dense: true,
              visualDensity: VisualDensity.compact,
              title: Text(
                'Could not search community people.',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
              trailing: TextButton(
                onPressed: onRetry == null
                    ? null
                    : () => _runComposerAction(onRetry!),
                child: const Text('Retry'),
              ),
            );
          }
          final candidate = suggestions[index];
          final name = candidate.pickerLabel;
          final avatarUrl =
              candidate.avatarUrl ?? userCache[candidate.pubkey]?.avatarUrl;

          final available = !unavailable.contains(candidate.pubkey);
          return ListTile(
            enabled: available,
            dense: true,
            visualDensity: VisualDensity.compact,
            leading: AvatarImage(
              imageUrl: avatarUrl,
              radius: 18,
              backgroundColor: context.controlSurface,
              fallback: Text(
                // Name-derived for named candidates; keyed to the hex
                // public key for unnamed ones so the compact-npub label
                // doesn't render `N` for everyone.
                candidate.initial,
                style: context.textTheme.labelMedium?.copyWith(
                  color: context.colors.onPrimaryContainer,
                  fontWeight: FontWeight.w600,
                ),
              ),
              isAgent: candidate.isAgent,
            ),
            title: Row(
              children: [
                Flexible(
                  child: Text(
                    name,
                    style: context.textTheme.titleSmall,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
                VerifiedMark(pubkey: candidate.pubkey),
              ],
            ),
            subtitle: _MentionSuggestionInfo.build(
              context,
              candidate: candidate,
              currentPubkey: currentPubkey,
              isDmChannel: isDmChannel,
              userCache: userCache,
            ),
            onTap: available
                ? () => _runComposerAction(() => onSelect(candidate))
                : null,
          );
        },
      ),
    );
  }
}

/// The secondary info line under a mention suggestion — mirrors desktop's
/// `MentionAutocomplete` subtitle: bot icon + "agent" (or an "admin" badge
/// for human admins), then "managed by …" / "not in channel" (or "not in DM").
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
    final notInChannel = !candidate.isMember;
    final outside = isDmChannel ? 'not in DM' : 'not in channel';
    final isAdmin = !candidate.isAgent && candidate.role == 'admin';

    final String? detail;
    if (ownerLabel != null && notInChannel) {
      detail = 'managed by $ownerLabel \u00b7 $outside';
    } else if (ownerLabel != null) {
      detail = 'managed by $ownerLabel';
    } else if (notInChannel) {
      detail = outside;
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
          Icon(BuzzIcons.bot, size: 12, color: context.colors.onSurfaceVariant),
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
    return Material(
      key: const ValueKey('channel-suggestions-popover'),
      type: MaterialType.card,
      color: appPopoverColor(context),
      surfaceTintColor: Colors.transparent,
      elevation: appPopoverElevation,
      shadowColor: appPopoverShadowColor(context),
      shape: appPopoverShape(context),
      clipBehavior: Clip.hardEdge,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 240),
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
                  BuzzIcons.hash,
                  size: 20,
                  color: context.colors.onSurfaceVariant,
                ),
              ),
              title: Text(channel.name, style: context.textTheme.bodyLarge),
              onTap: () => _runComposerAction(() => onSelect(channel)),
            );
          },
        ),
      ),
    );
  }
}
