import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_titled_sheet_layout.dart';

/// Uses the registry's name and private-instructions contract. Runtime/model
/// selection belongs to the remote service and is not an API option here.
class CreateAgentSheet extends HookConsumerWidget {
  const CreateAgentSheet({
    required this.controller,
    required this.community,
    required this.onCreated,
    super.key,
  });
  final RemoteAgentController controller;
  final Community community;
  final VoidCallback onCreated;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final name = useTextEditingController(
      text: controller.pending?['name'] as String? ?? '',
    );
    final instructions = useTextEditingController(
      text: controller.pending?['instructions'] as String? ?? '',
    );
    useListenable(name);
    final error = useState<String?>(null);
    final active = ref.watch(activeCommunityProvider).asData?.value;
    final sameCommunity = active?.id == community.id;
    final pending = controller.pending != null;
    final instructionsMissing =
        controller.pending?['instructionsMissing'] == true;
    final canCreate =
        !controller.busy && sameCommunity && name.text.trim().isNotEmpty;
    final actionStyle = IconButton.styleFrom(
      minimumSize: const Size.square(44),
      backgroundColor: context.colors.surfaceContainerHighest,
      side: BorderSide(color: context.colors.outlineVariant),
      shape: const CircleBorder(),
    );
    InputDecoration fieldDecoration(String label, {String? hint}) =>
        InputDecoration(
          labelText: label,
          floatingLabelBehavior: FloatingLabelBehavior.always,
          hintText: hint,
          filled: true,
          fillColor: context.colors.surfaceContainerHighest,
          contentPadding: const EdgeInsets.symmetric(
            horizontal: Grid.xs,
            vertical: Grid.xs,
          ),
          border: OutlineInputBorder(
            borderRadius: BorderRadius.circular(Radii.container),
            borderSide: BorderSide.none,
          ),
          enabledBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(Radii.container),
            borderSide: BorderSide.none,
          ),
          focusedBorder: OutlineInputBorder(
            borderRadius: BorderRadius.circular(Radii.container),
            borderSide: BorderSide.none,
          ),
        );

    Future<void> create() async {
      error.value = null;
      if (!sameCommunity) {
        error.value = 'The active community changed. Reopen Agents.';
        return;
      }
      final success = await controller.create(
        community,
        name.text,
        instructions: instructions.text,
      );
      if (!context.mounted) return;
      if (success) {
        onCreated();
        Navigator.pop(context, true);
      } else {
        error.value = controller.status;
      }
    }

    return PopScope(
      canPop: !controller.busy,
      child: Padding(
        padding: EdgeInsets.only(
          bottom: MediaQuery.viewInsetsOf(context).bottom,
        ),
        child: BuzzTitledSheetLayout(
          title: 'Create agent',
          leading: IconButton(
            tooltip: 'Cancel',
            style: actionStyle,
            onPressed: controller.busy ? null : () => Navigator.pop(context),
            icon: const Icon(Icons.close),
          ),
          trailing: IconButton(
            tooltip: pending ? 'Resume' : 'Create',
            style: actionStyle.copyWith(
              backgroundColor: WidgetStatePropertyAll(
                canCreate
                    ? context.colors.primary
                    : context.colors.surfaceContainerHighest,
              ),
              foregroundColor: WidgetStatePropertyAll(
                canCreate
                    ? context.colors.onPrimary
                    : context.colors.onSurfaceVariant,
              ),
            ),
            onPressed: canCreate ? create : null,
            icon: const Icon(Icons.check),
          ),
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(Grid.gutter),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                TextField(
                  controller: name,
                  autofocus: true,
                  enabled: !controller.busy && !pending,
                  textCapitalization: TextCapitalization.words,
                  decoration: fieldDecoration('Name'),
                ),
                const SizedBox(height: Grid.sm),
                TextField(
                  controller: instructions,
                  enabled:
                      !controller.busy && (!pending || instructionsMissing),
                  minLines: 4,
                  maxLines: 6,
                  textCapitalization: TextCapitalization.sentences,
                  textInputAction: TextInputAction.newline,
                  decoration: fieldDecoration(
                    'Instructions',
                    hint: 'Describe what this agent should do (optional).',
                  ),
                ),
                if (controller.busy) ...[
                  const SizedBox(height: Grid.xs),
                  const LinearProgressIndicator(),
                  Text(controller.status),
                ],
                if (error.value != null) ...[
                  const SizedBox(height: Grid.xs),
                  Text(
                    error.value!,
                    style: TextStyle(color: context.colors.error),
                  ),
                ],
                if (pending &&
                    controller.pending?['pubkey'] == null &&
                    !controller.busy)
                  TextButton(
                    onPressed: () async {
                      error.value = null;
                      if (await controller.discardPending()) {
                        name.clear();
                        instructions.clear();
                      } else if (context.mounted) {
                        error.value = controller.status;
                      }
                    },
                    child: const Text('Discard unfinished setup'),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
