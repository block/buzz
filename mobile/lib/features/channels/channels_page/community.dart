part of '../channels_page.dart';

class _CommunitySwitcherSheet extends ConsumerWidget {
  const _CommunitySwitcherSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final communitiesAsync = ref.watch(communityListProvider);
    final activeAsync = ref.watch(activeCommunityProvider);
    final sessionState = ref.watch(relaySessionProvider);

    return SafeArea(
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
          return Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final community in communities)
                _CommunitySwitcherTile(
                  community: community,
                  isActive: community.id == activeId,
                  sessionStatus: community.id == activeId
                      ? sessionState.status
                      : null,
                  onTap: () async {
                    if (community.id != activeId) {
                      await ref
                          .read(communityListProvider.notifier)
                          .switchCommunity(community.id);
                    }
                    if (context.mounted) Navigator.of(context).pop();
                  },
                  onRename: () async {
                    final nav = Navigator.of(context, rootNavigator: true);
                    final notifier = ref.read(communityListProvider.notifier);
                    Navigator.of(context).pop();
                    final name = await showDialog<String>(
                      context: nav.context,
                      useRootNavigator: true,
                      builder: (_) =>
                          _RenameCommunityDialog(currentName: community.name),
                    );
                    if (name != null && name.isNotEmpty) {
                      await notifier.renameCommunity(community.id, name);
                    }
                  },
                  onRemove: () async {
                    final confirmed = await showDialog<bool>(
                      context: context,
                      builder: (dialogContext) => AlertDialog(
                        title: const Text('Remove Community'),
                        content: Text(
                          'Remove "${community.name}"? You can re-pair later.',
                        ),
                        actions: [
                          TextButton(
                            onPressed: () =>
                                Navigator.of(dialogContext).pop(false),
                            child: const Text('Cancel'),
                          ),
                          TextButton(
                            onPressed: () =>
                                Navigator.of(dialogContext).pop(true),
                            child: const Text('Remove'),
                          ),
                        ],
                      ),
                    );
                    if (confirmed == true && context.mounted) {
                      final messenger = ScaffoldMessenger.of(context);
                      try {
                        await ref
                            .read(communityListProvider.notifier)
                            .removeCommunity(community.id);
                        if (context.mounted) Navigator.of(context).pop();
                      } catch (e) {
                        messenger.showSnackBar(
                          SnackBar(
                            content: Text('Failed to remove community: $e'),
                          ),
                        );
                      }
                    }
                  },
                ),
              const Divider(height: 1),
              ListTile(
                leading: const Icon(LucideIcons.plus),
                title: const Text('Add Community'),
                onTap: () {
                  final nav = Navigator.of(context, rootNavigator: true);
                  ref.read(pairingProvider.notifier).reset();
                  Navigator.of(context).pop();
                  nav.push(
                    MaterialPageRoute<void>(
                      builder: (_) => const PairingPage(addingCommunity: true),
                    ),
                  );
                },
              ),
            ],
          );
        },
      ),
    );
  }
}

class _CommunitySwitcherTile extends StatelessWidget {
  final Community community;
  final bool isActive;
  final SessionStatus? sessionStatus;
  final VoidCallback onTap;
  final VoidCallback onRename;
  final VoidCallback onRemove;

  const _CommunitySwitcherTile({
    required this.community,
    required this.isActive,
    required this.sessionStatus,
    required this.onTap,
    required this.onRename,
    required this.onRemove,
  });

  @override
  Widget build(BuildContext context) {
    final host = Uri.tryParse(community.relayUrl)?.host ?? community.relayUrl;

    return ListTile(
      leading: _StatusDot(isActive: isActive, sessionStatus: sessionStatus),
      title: Text(
        community.name,
        style: context.textTheme.bodyLarge?.copyWith(
          fontWeight: isActive ? FontWeight.w600 : FontWeight.normal,
        ),
      ),
      subtitle: Text(
        host,
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
      trailing: PopupMenuButton<String>(
        icon: Icon(
          LucideIcons.ellipsisVertical,
          size: 18,
          color: context.colors.onSurfaceVariant,
        ),
        onSelected: (value) {
          switch (value) {
            case 'rename':
              onRename();
            case 'remove':
              onRemove();
          }
        },
        itemBuilder: (_) => [
          const PopupMenuItem(value: 'rename', child: Text('Rename')),
          const PopupMenuItem(value: 'remove', child: Text('Remove')),
        ],
      ),
      onTap: onTap,
    );
  }
}

class _StatusDot extends StatelessWidget {
  final bool isActive;
  final SessionStatus? sessionStatus;

  const _StatusDot({required this.isActive, required this.sessionStatus});

  @override
  Widget build(BuildContext context) {
    if (!isActive) {
      return Container(
        width: 10,
        height: 10,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          color: context.colors.outline.withValues(alpha: 0.3),
        ),
      );
    }

    final color = switch (sessionStatus) {
      SessionStatus.connected => context.appColors.success,
      SessionStatus.connecting ||
      SessionStatus.reconnecting => context.appColors.warning,
      _ => context.colors.outline,
    };

    return Container(
      width: 10,
      height: 10,
      decoration: BoxDecoration(shape: BoxShape.circle, color: color),
    );
  }
}

class _RenameCommunityDialog extends HookWidget {
  final String currentName;

  const _RenameCommunityDialog({required this.currentName});

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: currentName);

    return AlertDialog(
      title: const Text('Rename Community'),
      content: TextField(
        controller: controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (value) {
          final name = value.trim();
          if (name.isNotEmpty) Navigator.of(context).pop(name);
        },
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () {
            final name = controller.text.trim();
            if (name.isNotEmpty) Navigator.of(context).pop(name);
          },
          child: const Text('Rename'),
        ),
      ],
    );
  }
}

class _CommunityIndicator extends ConsumerWidget {
  final VoidCallback onTap;

  const _CommunityIndicator({required this.onTap});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activeAsync = ref.watch(activeCommunityProvider);

    final name = activeAsync.value?.name;

    return GestureDetector(
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.only(left: _kCommunityAvatarInset),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _CommunityAvatar(name: name),
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
      ),
    );
  }
}

class _CommunityAvatar extends StatelessWidget {
  final String? name;

  const _CommunityAvatar({required this.name});

  @override
  Widget build(BuildContext context) {
    final trimmedName = name?.trim();
    final initial = trimmedName != null && trimmedName.isNotEmpty
        ? trimmedName.substring(0, 1).toUpperCase()
        : '?';

    return CircleAvatar(
      radius: 16,
      backgroundColor: context.colors.primaryContainer,
      child: Text(
        initial,
        style: context.textTheme.labelMedium?.copyWith(
          color: context.colors.onPrimaryContainer,
          fontWeight: FontWeight.w600,
        ),
      ),
    );
  }
}
