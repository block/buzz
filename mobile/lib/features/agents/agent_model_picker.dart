import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/sheet_action_section.dart';
import '../../shared/widgets/buzz_setting_menu.dart';

/// Uses the server catalog for both menus, including model-specific efforts.
class AgentModelPicker extends StatelessWidget {
  const AgentModelPicker({
    required this.controller,
    required this.catalog,
    required this.onRetry,
    required this.onChanged,
    this.modelId,
    this.effort,
    super.key,
  });
  final RemoteAgentController controller;
  final AsyncSnapshot<List<RemoteAgentModel>> catalog;
  final VoidCallback onRetry;
  final String? modelId;
  final String? effort;
  final void Function(String?, String?) onChanged;

  @override
  Widget build(BuildContext context) {
    // While loading, show the same rows disabled so nothing shifts.
    final loading = catalog.connectionState != ConnectionState.done;
    if (!loading && (catalog.hasError || catalog.data!.isEmpty)) {
      return Column(
        children: [
          Text(
            catalog.hasError
                ? 'Could not load available models.'
                : 'No models are available.',
          ),
          TextButton(onPressed: onRetry, child: const Text('Retry models')),
        ],
      );
    }
    final models = loading ? const <RemoteAgentModel>[] : catalog.data!;
    final selected = models.where((model) => model.id == modelId).firstOrNull;
    final validEffort = selected?.efforts.contains(effort) == true
        ? effort
        : null;
    final enabled = !loading && !controller.busy;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SheetActionSection(
          dividerIndent: Grid.xs,
          children: [
            BuzzSettingMenu<String>(
              controlKey: const Key('agent-model'),
              enabled: enabled,
              label: 'Mode',
              grouped: true,
              trailingIcon: Icons.unfold_more,
              onSelected: (value) => onChanged(value, null),
              options: [
                for (final model in models)
                  BuzzSettingMenuOption(label: model.name, value: model.id),
              ],
              value: selected?.id,
              valueLabel: loading ? 'Loading…' : selected?.name ?? 'Choose',
            ),

            BuzzSettingMenu<String>(
              controlKey: const Key('agent-effort'),
              enabled: enabled && selected != null,
              label: 'Speed',
              grouped: true,
              trailingIcon: Icons.unfold_more,
              onSelected: (value) => onChanged(selected!.id, value),
              options: [
                for (final value in selected?.efforts ?? <String>[])
                  BuzzSettingMenuOption(label: value, value: value),
              ],
              value: validEffort,
              valueLabel: loading ? 'Loading…' : validEffort ?? 'Choose',
            ),
          ],
        ),
        if (modelId != null && selected == null ||
            effort != null && validEffort == null)
          const Text(
            'Choose an available model and supported reasoning effort.',
          ),
      ],
    );
  }
}

/// Shares catalog loading with the submit action so stale selections cannot save.
(AsyncSnapshot<List<RemoteAgentModel>>, VoidCallback) useAgentModelCatalog(
  RemoteAgentController controller,
) {
  final retry = useState(0);
  final future = useMemoized(controller.modelCatalog, [
    controller,
    retry.value,
  ]);
  return (useFuture(future, preserveState: false), () => retry.value++);
}

/// True only after the loaded catalog confirms this exact pair.
bool validAgentModelSelection(
  AsyncSnapshot<List<RemoteAgentModel>> catalog,
  String? model,
  String? effort,
) =>
    catalog.connectionState == ConnectionState.done &&
    !catalog.hasError &&
    catalog.data?.any(
          (entry) => entry.id == model && entry.efforts.contains(effort),
        ) ==
        true;
