import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community.dart';
import 'agent_model_menu.dart';

/// An authenticated model menu with explicit reasoning effort confirmation.
class RemoteAgentModelControl extends HookWidget {
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
  Widget build(BuildContext context) {
    useListenable(controller);
    final retry = useState(0);
    final subject = controller.account.account?.subject;
    final future = useMemoized(controller.modelCatalog, [subject, retry.value]);
    final catalog = useFuture(future, preserveState: false);
    if (catalog.hasError) {
      return TextButton(
        onPressed: () => retry.value++,
        child: const Text('Retry models'),
      );
    }
    final models = catalog.data;
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
                            saved ? 'Model updated' : controller.status,
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
