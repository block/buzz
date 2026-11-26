import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/me/me_sync.dart';
import '../channel_sections/channel_sections_provider.dart';
import '../../../shared/me/me_recovery.dart';
import 'me_recovery_dialog.dart';
import 'me_settings_sheet.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/widgets/feedback_snackbar.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
import '../../../shared/widgets/modal_presentation.dart';
import '../../../shared/widgets/app_list.dart';
import '../../../shared/widgets/app_list_card.dart';
import '../../../shared/theme/theme.dart';

/// The workspace control shared by draft and existing Solo conversations.
({
  Widget action,
  IosNavigationAction nativeAction,
  bool busy,
  String name,
  Future<void> Function({MeSettingsPresenter? present})? openPicker,
})
useMeWorkspace(
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
  // Desktop's unified workspaces own filing once they exist; show that
  // placement read-only rather than the legacy Solo record's.
  final unified = enabled
      ? ref.watch(channelSectionsProvider.select((s) => !s.editable))
      : false;
  final layout = unified ? ref.watch(sidebarLayoutProvider) : null;
  final current = layout != null
      ? (channelId == null ? null : layout.assignments[channelId])
      : channelId == null
      ? selected
      : groups?.assignments[channelId];
  final name = layout != null
      ? layout.section(current ?? '')?.name
      : groups?.groups
                .where((group) => group['id'] == current)
                .firstOrNull?['name']
            as String?;
  final label = current == null
      ? 'Choose workspace'
      : 'Workspace: ${name ?? "Unavailable workspace"}';
  Future<void> choose(String picked) async {
    // Read now: a menu opened before the unified record arrived is stale.
    if (sync == null ||
        busy.value ||
        !ref.read(channelSectionsProvider).editable) {
      return;
    }
    if (picked == '+reload') {
      ref.invalidate(meGroupsProvider);
      return;
    }
    busy.value = true;
    try {
      sync.check();
      final project = picked.isEmpty ? null : picked;
      if (channelId == null) {
        onSelected?.call(project);
      } else {
        await sync.update(
          mayFile: () => ref.read(channelSectionsProvider).editable,
          (groups) => groups.place(
            channelId,
            group: project,
            clearGroup: project == null,
          ),
        );
      }
    } catch (error) {
      if (context.mounted && error is MeRecoveryConflict) {
        await offerMeRecovery(context, sync);
      } else if (context.mounted) {
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
      !unified &&
      !locked &&
      !busy.value &&
      (groups != null || groupState?.hasError == true);
  return (
    busy: busy.value,
    name: current == null ? 'No workspace' : name ?? 'Unavailable workspace',
    openPicker: !available
        ? null
        : ({MeSettingsPresenter? present}) async {
            Widget content(ValueChanged<String> onSelected) => Material(
              color: Colors.transparent,
              child: SafeArea(
                child: SingleChildScrollView(
                  child: AppListCard(
                    verticalPadding: Grid.twelve,
                    children: [
                      AppListRow(
                        title: 'No Workspace',
                        trailing: current == null
                            ? const Icon(BuzzIcons.check)
                            : null,
                        onTap: () => onSelected(''),
                      ),
                      for (final group
                          in groups?.groups ?? <Map<String, dynamic>>[])
                        AppListRow(
                          title: group['name'] as String,
                          trailing: current == group['id']
                              ? const Icon(BuzzIcons.check)
                              : null,
                          onTap: () => onSelected(group['id'] as String),
                        ),
                      if (groupState?.hasError == true)
                        AppListRow(
                          title: 'Reload workspaces',
                          onTap: () => onSelected('+reload'),
                        ),
                    ],
                  ),
                ),
              ),
            );
            final picked = present != null
                ? await present('Workspace', content)
                : await showBuzzModalBottomSheet<String>(
                    context: context,
                    title: 'Workspace',
                    showDragHandle: true,
                    builder: (sheetContext) =>
                        content((value) => Navigator.pop(sheetContext, value)),
                  );
            if (picked != null && context.mounted) await choose(picked);
          },
    action: PopupMenuButton<String>(
      key: const ValueKey('me-workspace-button'),
      tooltip: label,
      enabled: available,
      icon: const Icon(BuzzIcons.folder),
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
      ],
    ),
    nativeAction: IosNavigationAction(
      label: label,
      icon: BuzzIcons.folder,
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
            ],
    ),
  );
}
