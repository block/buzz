part of '../channels_page.dart';

class _CommunitySwitcherSheet extends HookConsumerWidget {
  const _CommunitySwitcherSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communitiesAsync = ref.watch(communityListProvider);
    final activeAsync = ref.watch(activeCommunityProvider);
    final isEditing = useState(false);

    return SafeArea(
      child: Column(
        key: const Key('community-switcher-sheet'),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              Grid.xxs,
              Grid.gutter,
              Grid.xxs,
            ),
            child: ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 32),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      'Switch Community',
                      key: const Key('community-switcher-title'),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.titleLarge?.copyWith(
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                  Padding(
                    padding: const EdgeInsets.only(right: Grid.half),
                    child: ConstrainedBox(
                      key: const Key('community-switcher-edit'),
                      constraints: const BoxConstraints(
                        minWidth: 48,
                        minHeight: 32,
                      ),
                      child: TextButton(
                        onPressed: () => isEditing.value = !isEditing.value,
                        style: TextButton.styleFrom(
                          minimumSize: Size.zero,
                          padding: const EdgeInsets.symmetric(
                            horizontal: Grid.half,
                          ),
                          tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                        ),
                        child: Text(isEditing.value ? 'Done' : 'Edit'),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
          Flexible(
            child: communitiesAsync.when(
              loading: () => const SizedBox(
                height: 120,
                child: Center(child: CircularProgressIndicator()),
              ),
              error: (e, _) => Padding(
                padding: const EdgeInsets.all(Grid.xs),
                child: Text('Error loading communities: $e'),
              ),
              data: (communities) {
                final activeId = activeAsync.value?.id;
                return SingleChildScrollView(
                  padding: const EdgeInsets.only(bottom: Grid.xs),
                  child: Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.gutter,
                    ),
                    child: Material(
                      key: const Key('community-switcher-options'),
                      color: context.colors.surfaceContainerHighest,
                      borderRadius: BorderRadius.circular(Radii.card),
                      clipBehavior: Clip.antiAlias,
                      child: Column(
                        children: [
                          for (
                            var index = 0;
                            index < communities.length;
                            index++
                          ) ...[
                            if (index > 0) const _CommunitySwitcherDivider(),
                            _CommunitySwitcherTile(
                              community: communities[index],
                              isActive: communities[index].id == activeId,
                              isEditing: isEditing.value,
                              onTap: isEditing.value
                                  ? null
                                  : () async {
                                      final community = communities[index];
                                      if (community.id != activeId) {
                                        await ref
                                            .read(
                                              communityListProvider.notifier,
                                            )
                                            .switchCommunity(community.id);
                                      }
                                      if (context.mounted) {
                                        Navigator.of(context).pop();
                                      }
                                    },
                              onRemove: () => _confirmRemoveCommunity(
                                context,
                                ref,
                                communities[index],
                                closeSheetAfterRemoval:
                                    communities[index].id == activeId,
                              ),
                            ),
                          ],
                          if (communities.isNotEmpty)
                            const _CommunitySwitcherDivider(),
                          _AddCommunityTile(
                            onTap: () {
                              final nav = Navigator.of(
                                context,
                                rootNavigator: true,
                              );
                              ref.read(pairingProvider.notifier).reset();
                              Navigator.of(context).pop();
                              nav.push(
                                MaterialPageRoute<void>(
                                  builder: (_) =>
                                      const PairingPage(addingCommunity: true),
                                ),
                              );
                            },
                          ),
                        ],
                      ),
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}

const double _communitySwitcherAvatarSize = 36;

class _CommunitySwitcherDivider extends StatelessWidget {
  const _CommunitySwitcherDivider();

  @override
  Widget build(BuildContext context) {
    return Divider(
      height: 1,
      thickness: 1,
      indent: Grid.xs + _communitySwitcherAvatarSize + Grid.xs,
      endIndent: 0,
      color: context.colors.onSurface.withValues(alpha: 0.12),
    );
  }
}

class _CommunitySwitcherTile extends StatelessWidget {
  final Community community;
  final bool isActive;
  final bool isEditing;
  final VoidCallback? onTap;
  final VoidCallback onRemove;

  const _CommunitySwitcherTile({
    required this.community,
    required this.isActive,
    required this.isEditing,
    required this.onTap,
    required this.onRemove,
  });

  @override
  Widget build(BuildContext context) {
    final host = Uri.tryParse(community.relayUrl)?.host ?? community.relayUrl;

    return InkWell(
      key: Key('community-switcher-row-${community.id}'),
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.xs,
          Grid.twelve,
          Grid.xxs,
          Grid.twelve,
        ),
        child: Row(
          children: [
            _CommunityAvatar(
              key: Key('community-switcher-avatar-${community.id}'),
              name: community.name,
              relayUrl: community.relayUrl,
              size: _communitySwitcherAvatarSize,
            ),
            const SizedBox(width: Grid.xs),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    community.name,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.bodyLarge?.copyWith(
                      fontWeight: isActive
                          ? FontWeight.w600
                          : FontWeight.normal,
                    ),
                  ),
                  const SizedBox(height: Grid.quarter),
                  Text(
                    host,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: Grid.xxs),
            _CommunityActionSwap(
              key: Key('community-switcher-action-${community.id}'),
              communityId: community.id,
              communityName: community.name,
              isActive: isActive,
              isEditing: isEditing,
              onRemove: onRemove,
            ),
          ],
        ),
      ),
    );
  }
}

const _communityActionSwapDuration = Duration(milliseconds: 250);
const double _communityActionSwapBlur = 2;
const double _communityActionSwapStartScale = 0.25;

class _CommunityActionSwap extends StatelessWidget {
  const _CommunityActionSwap({
    super.key,
    required this.communityId,
    required this.communityName,
    required this.isActive,
    required this.isEditing,
    required this.onRemove,
  });

  final String communityId;
  final String communityName;
  final bool isActive;
  final bool isEditing;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final reduceMotion =
        MediaQuery.maybeOf(context)?.disableAnimations ?? false;

    return SizedBox.square(
      dimension: 40,
      child: AnimatedSwitcher(
        duration: reduceMotion ? Duration.zero : _communityActionSwapDuration,
        reverseDuration: reduceMotion
            ? Duration.zero
            : _communityActionSwapDuration,
        transitionBuilder: (child, animation) {
          final eased = CurvedAnimation(
            parent: animation,
            curve: Curves.easeInOut,
          );
          return AnimatedBuilder(
            animation: eased,
            child: child,
            builder: (context, child) {
              final progress = eased.value;
              final blur = _communityActionSwapBlur * (1 - progress);
              final scale =
                  _communityActionSwapStartScale +
                  ((1 - _communityActionSwapStartScale) * progress);
              return Opacity(
                opacity: progress,
                child: ImageFiltered(
                  enabled: blur > 0.01,
                  imageFilter: ImageFilter.blur(sigmaX: blur, sigmaY: blur),
                  child: Transform.scale(scale: scale, child: child),
                ),
              );
            },
          );
        },
        child: isEditing
            ? IconButton(
                key: ValueKey('community-switcher-remove-$communityId'),
                tooltip: 'Remove $communityName',
                visualDensity: VisualDensity.compact,
                onPressed: onRemove,
                icon: Icon(
                  LucideIcons.trash2,
                  size: 18,
                  color: context.colors.error,
                ),
              )
            : Center(
                key: ValueKey('community-switcher-selection-$communityId'),
                child: _CommunitySelectionIndicator(
                  key: ValueKey('community-switcher-circle-$communityId'),
                  communityName: communityName,
                  isActive: isActive,
                ),
              ),
      ),
    );
  }
}

class _CommunitySelectionIndicator extends StatelessWidget {
  const _CommunitySelectionIndicator({
    super.key,
    required this.communityName,
    required this.isActive,
  });

  final String communityName;
  final bool isActive;

  @override
  Widget build(BuildContext context) {
    return Semantics(
      label: isActive
          ? '$communityName is the current community'
          : 'Switch to $communityName',
      child: Container(
        width: 24,
        height: 24,
        decoration: BoxDecoration(
          color: isActive ? context.appColors.success : Colors.transparent,
          border: isActive
              ? null
              : Border.all(color: context.colors.outlineVariant, width: 2),
          shape: BoxShape.circle,
        ),
        child: isActive
            ? const Icon(LucideIcons.check, color: Colors.white, size: 16)
            : null,
      ),
    );
  }
}

class _AddCommunityTile extends StatelessWidget {
  const _AddCommunityTile({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return InkWell(
      key: const Key('community-switcher-add'),
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: Grid.xs,
          vertical: Grid.twelve,
        ),
        child: Row(
          children: [
            Container(
              width: _communitySwitcherAvatarSize,
              height: _communitySwitcherAvatarSize,
              decoration: BoxDecoration(
                color: context.colors.primaryContainer,
                shape: BoxShape.circle,
              ),
              child: Icon(
                LucideIcons.plus,
                size: 18,
                color: context.colors.onPrimaryContainer,
              ),
            ),
            const SizedBox(width: Grid.xs),
            Text(
              'Add Community',
              style: context.textTheme.bodyLarge?.copyWith(
                fontWeight: FontWeight.w500,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

Future<void> _confirmRemoveCommunity(
  BuildContext context,
  WidgetRef ref,
  Community community, {
  required bool closeSheetAfterRemoval,
}) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (dialogContext) => AlertDialog.adaptive(
      title: const Text('Remove community?'),
      content: Text(
        'Are you sure you want to remove “${community.name}”? '
        'You can pair with it again later.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(dialogContext).pop(true),
          style: FilledButton.styleFrom(backgroundColor: context.colors.error),
          child: const Text('Remove'),
        ),
      ],
    ),
  );

  if (confirmed != true || !context.mounted) return;

  final messenger = ScaffoldMessenger.of(context);
  try {
    await ref
        .read(communityListProvider.notifier)
        .removeCommunity(community.id);
    if (closeSheetAfterRemoval && context.mounted) {
      Navigator.of(context).pop();
    }
  } catch (e) {
    messenger.showSnackBar(
      SnackBar(content: Text('Failed to remove community: $e')),
    );
  }
}

class _CommunityIndicator extends ConsumerWidget {
  final VoidCallback onTap;

  const _CommunityIndicator({required this.onTap});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activeAsync = ref.watch(activeCommunityProvider);

    final activeCommunity = activeAsync.value;
    final name = activeCommunity?.name;

    return GestureDetector(
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          _CommunityAvatar(name: name, relayUrl: activeCommunity?.relayUrl),
          const SizedBox(width: Grid.xxs),
          if (name != null)
            Flexible(
              child: Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: context.textTheme.labelLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                ),
              ),
            )
          else
            Text(
              'Community',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.labelLarge?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
        ],
      ),
    );
  }
}

class _CommunityAvatar extends ConsumerWidget {
  final String? name;
  final String? relayUrl;
  final double size;

  const _CommunityAvatar({
    super.key,
    required this.name,
    this.relayUrl,
    this.size = 32,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final trimmedName = name?.trim();
    final initial = trimmedName != null && trimmedName.isNotEmpty
        ? trimmedName.substring(0, 1).toUpperCase()
        : '?';
    final relay = relayUrl;
    final iconUrl = relay == null
        ? null
        : ref.watch(communityIconProvider(relay)).value;

    return AvatarImage(
      imageUrl: iconUrl,
      radius: size / 2,
      backgroundColor: context.colors.primaryContainer,
      fallback: Text(
        initial,
        style: context.textTheme.labelMedium?.copyWith(
          color: context.colors.onPrimaryContainer,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}
