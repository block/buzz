import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/me/me_sync.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/feedback_snackbar.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
import '../../../shared/widgets/modal_presentation.dart';
import 'new_me_project.dart';

/// The workspace control shared by draft and existing Solo conversations.
({Widget action, IosNavigationAction nativeAction, bool busy}) useMeWorkspace(
  BuildContext context,
  WidgetRef ref, {
  required bool enabled,
  String? channelId,
  String? selected,
  ValueChanged<String?>? onSelected,
  bool locked = false,
}) {
  final busy = useState(false);
  final sync = enabled ? ref.watch(meSyncProvider) : null;
  final groups = enabled ? ref.watch(meGroupsProvider).value : null;
  final current = channelId == null ? selected : groups?.assignments[channelId];
  final name =
      groups?.groups
              .where((group) => group['id'] == current)
              .firstOrNull?['name']
          as String?;
  final label = current == null
      ? 'Choose workspace'
      : 'Workspace: ${name ?? "Unavailable workspace"}';
  Future<void> choose() async {
    if (sync == null || busy.value) return;
    busy.value = true;
    try {
      final picked = await showBuzzModalBottomSheet<String>(
        context: context,
        title: 'Workspace',
        isScrollControlled: true,
        showDragHandle: true,
        builder: (_) => _WorkspaceChoices(selected: current),
      );
      if (picked == null || !context.mounted) return;
      sync.check();
      final id = picked == '+create'
          ? await createMeProject(
              context,
              ref,
              workspace: true,
              channelId: channelId,
            )
          : picked;
      if (id == null || !context.mounted) return;
      sync.check();
      final project = id.isEmpty ? null : id;
      if (channelId == null) {
        onSelected?.call(project);
      } else if (picked != '+create') {
        await sync.update(
          (groups) => groups.place(
            channelId,
            group: project,
            clearGroup: project == null,
          ),
        );
      }
    } catch (error) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showFeedbackSnackBar(
          SnackBar(
            content: Text(error.toString().replaceFirst('Bad state: ', '')),
          ),
          kind: ToastKind.error,
        );
      }
    } finally {
      if (context.mounted) busy.value = false;
    }
  }

  final onPressed = !enabled || locked || busy.value ? null : choose;
  return (
    busy: busy.value,
    action: IconButton(
      key: const ValueKey('me-workspace-button'),
      tooltip: label,
      onPressed: onPressed,
      icon: const Icon(BuzzIcons.workspace),
    ),
    nativeAction: IosNavigationAction(
      label: label,
      icon: BuzzIcons.workspace,
      onPressed: onPressed,
    ),
  );
}

class _WorkspaceChoices extends ConsumerWidget {
  const _WorkspaceChoices({required this.selected});
  final String? selected;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final groups = ref.watch(meGroupsProvider);
    return SizedBox(
      height: (MediaQuery.sizeOf(context).height * 0.6).clamp(0.0, 420.0),
      child: Material(
        type: MaterialType.transparency,
        child: ListView(
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          children: [
            ListTile(
              title: const Text('No workspace'),
              selected: selected == null,
              trailing: selected == null ? const Icon(BuzzIcons.check) : null,
              onTap: () => Navigator.pop(context, ''),
            ),
            if (groups.isLoading) const LinearProgressIndicator(),
            if (groups.hasError)
              TextButton(
                onPressed: () => ref.invalidate(meGroupsProvider),
                child: const Text('Reload workspaces'),
              ),
            for (final group
                in groups.value?.groups ?? <Map<String, dynamic>>[])
              ListTile(
                title: Text(group['name'] as String),
                selected: selected == group['id'],
                trailing: selected == group['id']
                    ? const Icon(BuzzIcons.check)
                    : null,
                onTap: () => Navigator.pop(context, group['id'] as String),
              ),
            ListTile(
              leading: const Icon(BuzzIcons.plus),
              title: const Text('Create workspace'),
              onTap: () => Navigator.pop(context, '+create'),
            ),
          ],
        ),
      ),
    );
  }
}
