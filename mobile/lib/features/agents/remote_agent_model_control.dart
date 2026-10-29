import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community.dart';
import 'agent_model_menu.dart';

// Share one request/result while controls are mounted. Account transitions
// discard the old catalog; save-time validation still fetches a fresh catalog.
final _modelCatalogProvider = FutureProvider.autoDispose
    .family<List<RemoteAgentModel>, RemoteAgentController>((ref, controller) {
      final subject = controller.account.subjectChanges;
      void changed() => ref.invalidateSelf();
      subject.addListener(changed);
      ref.onDispose(() => subject.removeListener(changed));
      return controller.modelCatalog();
    }, retry: (_, _) => null);

/// An authenticated model menu with explicit reasoning effort confirmation.
class RemoteAgentModelControl extends HookConsumerWidget {
  const RemoteAgentModelControl({
    required this.controller,
    required this.agent,
    required this.community,
    required this.isCurrent,
    super.key,
  });
  final RemoteAgentController controller;
  final Map<String, dynamic> agent;
  final Community community;
  final bool Function() isCurrent;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final subject = controller.account.account?.subject;
    final provider = _modelCatalogProvider(controller);
    final catalog = ref.watch(provider);
    if (catalog.isLoading) {
      return const Center(child: CircularProgressIndicator());
    }
    if (catalog.hasError) {
      return TextButton(
        onPressed: () => ref.invalidate(provider),
        child: const Text('Retry models'),
      );
    }
    final models = catalog.asData?.value;
    if (models == null) return const Center(child: CircularProgressIndicator());
    if (models.isEmpty) return const Center(child: Text('No models available'));
    final selected = models
        .where((model) => model.id == agent['model_id'])
        .firstOrNull;
    return AgentModelMenu(
      label: selected?.name ?? (agent['model_id'] as String?) ?? 'Choose model',
      actions: [
        for (final model in models)
          (
            label: model.name,
            symbol: model.id == selected?.id ? 'checkmark' : '',
            onPressed: controller.busy
                ? null
                : () async {
                    String? effort =
                        model.efforts.contains(agent['reasoning_effort'])
                        ? agent['reasoning_effort'] as String
                        : null;
                    final choice = await showDialog<String>(
                      context: context,
                      builder: (context) => StatefulBuilder(
                        builder: (context, setState) => AlertDialog(
                          title: Text(model.name),
                          content: DropdownButtonFormField<String>(
                            initialValue: effort,
                            isExpanded: true,
                            decoration: const InputDecoration(
                              labelText: 'Reasoning effort',
                            ),
                            items: [
                              for (final value in model.efforts)
                                DropdownMenuItem(
                                  value: value,
                                  child: Text(value),
                                ),
                            ],
                            onChanged: (value) =>
                                setState(() => effort = value),
                          ),
                          actions: [
                            TextButton(
                              onPressed: () => Navigator.pop(context),
                              child: const Text('Cancel'),
                            ),
                            TextButton(
                              onPressed: effort == null
                                  ? null
                                  : () => Navigator.pop(context, effort),
                              child: const Text('Save'),
                            ),
                          ],
                        ),
                      ),
                    );
                    if (choice == null || !context.mounted) return;
                    final saved = await controller.updateModel(
                      community: community,
                      pubkey: agent['agent_pubkey'] as String,
                      model: model,
                      effort: choice,
                      isCurrent: () =>
                          context.mounted &&
                          controller.account.account?.subject == subject &&
                          isCurrent(),
                    );
                    if (context.mounted) {
                      ScaffoldMessenger.of(context).showSnackBar(
                        SnackBar(
                          content: Text(
                            saved ? 'Model updated' : controller.failure,
                          ),
                        ),
                      );
                    }
                  },
          ),
      ],
    );
  }
}
