part of '../channels_page.dart';

class _WorkspaceSwitcherSheet extends ConsumerWidget {
  const _WorkspaceSwitcherSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final workspacesAsync = ref.watch(workspaceListProvider);
    final activeAsync = ref.watch(activeWorkspaceProvider);
    final sessionState = ref.watch(relaySessionProvider);

    return SafeArea(
      child: workspacesAsync.when(
        loading: () => const SizedBox(
          height: 120,
          child: Center(child: CircularProgressIndicator()),
        ),
        error: (e, _) => Padding(
          padding: const EdgeInsets.all(Grid.xs),
          child: Text('Error loading workspaces: $e'),
        ),
        data: (workspaces) {
          final activeId = activeAsync.value?.id;
          return Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final workspace in workspaces)
                _WorkspaceSwitcherTile(
                  workspace: workspace,
                  isActive: workspace.id == activeId,
                  sessionStatus: workspace.id == activeId
                      ? sessionState.status
                      : null,
                  onTap: () async {
                    if (workspace.id != activeId) {
                      await ref
                          .read(workspaceListProvider.notifier)
                          .switchWorkspace(workspace.id);
                    }
                    if (context.mounted) Navigator.of(context).pop();
                  },
                  onRename: () async {
                    final nav = Navigator.of(context, rootNavigator: true);
                    final notifier = ref.read(workspaceListProvider.notifier);
                    Navigator.of(context).pop();
                    final name = await showDialog<String>(
                      context: nav.context,
                      useRootNavigator: true,
                      builder: (_) =>
                          _RenameWorkspaceDialog(currentName: workspace.name),
                    );
                    if (name != null && name.isNotEmpty) {
                      await notifier.renameWorkspace(workspace.id, name);
                    }
                  },
                  onRemove: () async {
                    final confirmed = await showDialog<bool>(
                      context: context,
                      builder: (dialogContext) => AlertDialog(
                        title: const Text('Remove Workspace'),
                        content: Text(
                          'Remove "${workspace.name}"? You can re-pair later.',
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
                            .read(workspaceListProvider.notifier)
                            .removeWorkspace(workspace.id);
                        if (context.mounted) Navigator.of(context).pop();
                      } catch (e) {
                        messenger.showSnackBar(
                          SnackBar(
                            content: Text('Failed to remove workspace: $e'),
                          ),
                        );
                      }
                    }
                  },
                ),
              const Divider(height: 1),
              ListTile(
                leading: const Icon(LucideIcons.plus),
                title: const Text('Add Workspace'),
                onTap: () {
                  final nav = Navigator.of(context, rootNavigator: true);
                  ref.read(pairingProvider.notifier).reset();
                  Navigator.of(context).pop();
                  nav.push(
                    MaterialPageRoute<void>(
                      builder: (_) => const PairingPage(addingWorkspace: true),
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

class _WorkspaceSwitcherTile extends StatelessWidget {
  final Workspace workspace;
  final bool isActive;
  final SessionStatus? sessionStatus;
  final VoidCallback onTap;
  final VoidCallback onRename;
  final VoidCallback onRemove;

  const _WorkspaceSwitcherTile({
    required this.workspace,
    required this.isActive,
    required this.sessionStatus,
    required this.onTap,
    required this.onRename,
    required this.onRemove,
  });

  @override
  Widget build(BuildContext context) {
    final host = Uri.tryParse(workspace.relayUrl)?.host ?? workspace.relayUrl;

    return ListTile(
      leading: _StatusDot(isActive: isActive, sessionStatus: sessionStatus),
      title: Text(
        workspace.name,
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

class _RenameWorkspaceDialog extends HookWidget {
  final String currentName;

  const _RenameWorkspaceDialog({required this.currentName});

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: currentName);

    return AlertDialog(
      title: const Text('Rename Workspace'),
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

class _WorkspaceIndicator extends ConsumerWidget {
  final VoidCallback onTap;

  const _WorkspaceIndicator({required this.onTap});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final activeAsync = ref.watch(activeWorkspaceProvider);

    final name = activeAsync.value?.name;

    return GestureDetector(
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.only(left: _kWorkspaceAvatarInset),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _WorkspaceAvatar(name: name),
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
                'Workspace',
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

class _WorkspaceAvatar extends StatelessWidget {
  final String? name;

  const _WorkspaceAvatar({required this.name});

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
