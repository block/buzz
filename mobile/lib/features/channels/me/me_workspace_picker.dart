import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/me/me_sync.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/feedback_snackbar.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
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
  final groupState = enabled ? ref.watch(meGroupsProvider) : null;
  final groups = groupState?.value;
  final current = channelId == null ? selected : groups?.assignments[channelId];
  final name =
      groups?.groups
              .where((group) => group['id'] == current)
              .firstOrNull?['name']
          as String?;
  final label = current == null
      ? 'Choose workspace'
      : 'Workspace: ${name ?? "Unavailable workspace"}';
  Future<void> choose(String picked) async {
    if (sync == null || busy.value) return;
    if (picked == '+reload') {
      ref.invalidate(meGroupsProvider);
      return;
    }
    busy.value = true;
    try {
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

  final available =
      enabled &&
      !locked &&
      !busy.value &&
      (groups != null || groupState?.hasError == true);
  return (
    busy: busy.value,
    action: PopupMenuButton<String>(
      key: const ValueKey('me-workspace-button'),
      tooltip: label,
      enabled: available,
      icon: const Icon(BuzzIcons.workspace),
      onSelected: choose,
      itemBuilder: (_) => [
        CheckedPopupMenuItem(
          value: '',
          checked: current == null,
          child: const Text('No Workspace'),
        ),
        for (final group in groups?.groups ?? <Map<String, dynamic>>[])
          CheckedPopupMenuItem(
            value: group['id'] as String,
            checked: current == group['id'],
            child: Text(group['name'] as String),
          ),
        if (groupState?.hasError == true)
          const PopupMenuItem(
            value: '+reload',
            child: Text('Reload workspaces'),
          ),
        const PopupMenuItem(value: '+create', child: Text('Create workspace')),
      ],
    ),
    nativeAction: IosNavigationAction(
      label: label,
      icon: BuzzIcons.workspace,
      sharesBackground: false,
      children: !available
          ? const []
          : [
              IosNavigationAction(
                label: 'No Workspace',
                selected: current == null,
                onPressed: () => choose(''),
              ),
              for (final group in groups?.groups ?? <Map<String, dynamic>>[])
                IosNavigationAction(
                  label: group['name'] as String,
                  selected: current == group['id'],
                  onPressed: () => choose(group['id'] as String),
                ),
              if (groupState?.hasError == true)
                IosNavigationAction(
                  label: 'Reload workspaces',
                  onPressed: () => choose('+reload'),
                ),
              IosNavigationAction(
                label: 'Create workspace',
                icon: BuzzIcons.plus,
                onPressed: () => choose('+create'),
              ),
            ],
    ),
  );
}
