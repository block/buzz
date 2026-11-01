import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/me/me_sync.dart';
import '../../../shared/me/me_groups.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/feedback_snackbar.dart';
import '../../../shared/widgets/ios_navigation_bar.dart';

/// Keeps workspace placement available from a Solo conversation's header.
({Widget action, IosNavigationAction nativeAction}) useMeWorkspacePicker(
  BuildContext context,
  WidgetRef ref, {
  required String channelId,
  required bool enabled,
}) {
  final busy = useState(false);
  final groups = enabled ? ref.watch(meGroupsProvider).value : null;
  final current = groups?.assignments[channelId];
  final label =
      groups?.groups.where((g) => g['id'] == current).firstOrNull?['name']
          as String? ??
      'Chats';
  Future<void> choose(String? value) async {
    if (busy.value) return;
    busy.value = true;
    try {
      await ref
          .read(meSyncProvider)
          .update(
            (groups) => value == null
                ? MeGroups(
                    groups: groups.groups,
                    channels: groups.channels,
                    assignments: Map<String, String>.of(groups.assignments)
                      ..remove(channelId),
                  )
                : groups.place(channelId, group: value),
          );
    } catch (error) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showFeedbackSnackBar(
          SnackBar(content: Text('$error')),
          kind: ToastKind.error,
        );
      }
    } finally {
      if (context.mounted) busy.value = false;
    }
  }

  final available = enabled && groups != null && !busy.value;
  return (
    nativeAction: IosNavigationAction(
      label: 'Workspace: $label',
      icon: BuzzIcons.folder,
      sharesBackground: false,
      children: !available
          ? const []
          : [
              IosNavigationAction(
                label: 'Chats',
                selected: current == null,
                onPressed: () => choose(null),
              ),
              for (final group in groups.groups)
                IosNavigationAction(
                  label: group['name'] as String,
                  selected: current == group['id'],
                  onPressed: () => choose(group['id'] as String),
                ),
            ],
    ),
    action: PopupMenuButton<String>(
      tooltip: 'Workspace: $label',
      icon: const Icon(BuzzIcons.folder),
      enabled: available,
      onSelected: (v) => choose(v.isEmpty ? null : v),
      itemBuilder: (_) => [
        CheckedPopupMenuItem(
          value: '',
          checked: current == null,
          child: const Text('Chats'),
        ),
        for (final group in groups?.groups ?? const <Map<String, dynamic>>[])
          CheckedPopupMenuItem(
            checked: current == group['id'],
            value: group['id'] as String,
            child: Text(group['name'] as String),
          ),
      ],
    ),
  );
}
