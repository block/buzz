import 'dart:math';
import 'agent_model_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/relay/relay.dart';
import 'agent_avatar_picker.dart';
import '../../shared/community/community.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import 'agent_avatar_editor_page.dart';
import '../../shared/widgets/sheet_action_section.dart';

/// Collects agent details, an avatar, and a service-supported model.
class CreateAgentPage extends HookConsumerWidget {
  const CreateAgentPage({
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
    ref.listen(remoteAgentsEnabledProvider, (_, enabled) {
      final route = ModalRoute.of(context);
      if (!enabled && route != null) {
        Navigator.of(context).removeRoute(route);
      }
    });
    final (catalog, retryCatalog) = useAgentModelCatalog(controller);
    final openingSubject = useMemoized(
      () => controller.account.account?.subject,
      const [],
    );
    final model = useState(controller.pending?['model_id'] as String?);
    final effort = useState(controller.pending?['reasoning_effort'] as String?);
    final name = useTextEditingController(
      text: controller.pending?['name'] as String? ?? '',
    );
    final instructions = useTextEditingController(
      text: controller.pending?['instructions'] as String? ?? '',
    );
    useListenable(name);
    final error = useState<String?>(null);
    final avatar = useState(
      AgentAvatarSelection.url(
        controller.pending?['picture'] as String? ??
            agentAvatarCatalog[Random().nextInt(agentAvatarCatalog.length)].url,
      ),
    );
    final preparing = useState(false);
    final busy = controller.busy || preparing.value;
    final active = ref.watch(activeCommunityProvider).asData?.value;
    final sameCommunity = active?.id == community.id;
    final pending = controller.pending != null;
    final instructionsMissing =
        controller.pending?['instructionsMissing'] == true;
    final canCreate =
        !busy &&
        sameCommunity &&
        name.text.trim().isNotEmpty &&
        validAgentModelSelection(catalog, model.value, effort.value);
    InputDecoration fieldDecoration(String label, {String? hint}) =>
        InputDecoration(
          hintText: hint,
          filled: false,
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
      if (controller.account.account?.subject != openingSubject) {
        error.value =
            'The account changed. Reopen Agents and choose a model again.';
        return;
      }
      if (!sameCommunity) {
        error.value = 'The active community changed. Reopen Agents.';
        return;
      }
      preparing.value = true;
      try {
        final subject = controller.account.account?.subject;
        final service = ref.read(mediaUploadServiceProvider);
        final picture = pending ? null : await avatar.value.prepare(service);
        if (!context.mounted) return;
        if (ref.read(activeCommunityProvider).asData?.value?.id !=
                community.id ||
            controller.account.account?.subject != subject) {
          error.value = 'The account or community changed. Reopen Agents.';
          return;
        }
        final success = await controller.create(
          community,
          name.text,
          instructions: instructions.text,
          pictureUrl: picture,
          modelId: model.value,
          reasoningEffort: effort.value,
        );
        if (!context.mounted) return;
        if (success) {
          onCreated();
          Navigator.pop(context, true);
        } else {
          error.value = controller.failure;
          retryCatalog();
        }
      } catch (_) {
        if (context.mounted) {
          error.value = 'Could not upload the avatar. Try again.';
        }
      } finally {
        if (context.mounted) preparing.value = false;
      }
    }

    return PopScope(
      canPop: !busy,
      child: FrostedScaffold(
        appBar: FrostedAppBar(
          nativeTitle: 'Create agent',
          title: const Text('Create agent'),
          automaticallyImplyLeading: false,
          nativeLeading: IosNavigationAction(
            label: 'Cancel',
            symbol: 'xmark',
            onPressed: busy ? null : () => Navigator.pop(context),
          ),
          nativeActions: [
            IosNavigationAction(
              label: pending ? 'Resume' : 'Create',
              symbol: 'checkmark',
              onPressed: canCreate ? create : null,
            ),
          ],
          leading: IconButton(
            tooltip: 'Cancel',
            onPressed: busy ? null : () => Navigator.pop(context),
            icon: const Icon(Icons.close),
          ),
          actions: [
            IconButton(
              tooltip: pending ? 'Resume' : 'Create',
              onPressed: canCreate ? create : null,
              icon: const Icon(Icons.check),
            ),
          ],
        ),
        body: SafeArea(
          top: false,
          child: SingleChildScrollView(
            padding: EdgeInsets.fromLTRB(
              Grid.gutter,
              frostedAppBarHeight(context) + Grid.md,
              Grid.gutter,
              Grid.md,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Center(
                  child: Column(
                    children: [
                      Semantics(
                        label: 'Agent avatar',
                        image: true,
                        child: SizedBox.square(
                          dimension: 160,
                          child: avatar.value.bytes != null
                              ? ClipOval(
                                  child: Image.memory(
                                    avatar.value.bytes!,
                                    fit: BoxFit.cover,
                                  ),
                                )
                              : PresetAgentAvatar(
                                  imageUrl: avatar.value.url!,
                                  size: 160,
                                  fallback: const Icon(
                                    Icons.smart_toy_outlined,
                                    size: 80,
                                  ),
                                ),
                        ),
                      ),
                      if (!pending)
                        _ChooseAvatarButton(
                          onPressed: busy
                              ? null
                              : () async {
                                  FocusScope.of(context).unfocus();
                                  final result = await Navigator.of(context)
                                      .push<AgentAvatarSelection>(
                                        MaterialPageRoute(
                                          builder: (_) => AgentAvatarEditorPage(
                                            initialSelection: avatar.value,
                                          ),
                                        ),
                                      );
                                  if (context.mounted && result != null) {
                                    avatar.value = result;
                                  }
                                },
                        ),
                    ],
                  ),
                ),
                const SizedBox(height: Grid.md),
                SheetActionSection(
                  label: 'Name',
                  children: [
                    Semantics(
                      label: 'Name',
                      child: TextField(
                        controller: name,
                        autofocus: false,
                        enabled: !busy && !pending,
                        textCapitalization: TextCapitalization.words,
                        decoration: fieldDecoration('Name'),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: Grid.sm),
                SheetActionSection(
                  label: 'Instructions',
                  children: [
                    Semantics(
                      label: 'Instructions',
                      child: TextField(
                        controller: instructions,
                        enabled: !busy && (!pending || instructionsMissing),
                        minLines: 4,
                        maxLines: 6,
                        textCapitalization: TextCapitalization.sentences,
                        textInputAction: TextInputAction.newline,
                        decoration: fieldDecoration(
                          'Instructions',
                          hint:
                              'Describe what this agent should do (optional).',
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: Grid.sm),
                AgentModelPicker(
                  controller: controller,
                  catalog: catalog,
                  onRetry: retryCatalog,
                  modelId: model.value,
                  effort: effort.value,
                  onChanged: (id, value) {
                    model.value = id;
                    effort.value = value;
                  },
                ),
                if (busy) ...[
                  const SizedBox(height: Grid.xs),
                  const LinearProgressIndicator(),
                ],
                if (error.value != null) ...[
                  const SizedBox(height: Grid.xs),
                  Text(
                    error.value!,
                    style: TextStyle(color: context.colors.error),
                  ),
                ],
                if (controller.canDiscardPending && !busy)
                  TextButton(
                    onPressed: () async {
                      error.value = null;
                      if (await controller.discardPending()) {
                        name.clear();
                        instructions.clear();
                      } else if (context.mounted) {
                        error.value = controller.failure;
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

/// Platform capsule action for entering the dedicated avatar editor.
class _ChooseAvatarButton extends StatelessWidget {
  const _ChooseAvatarButton({required this.onPressed});
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosGlassNavigationAction(
        label: 'Choose avatar',
        width: 160,
        onPressed: onPressed,
      );
    }
    return FilledButton.tonal(
      style: FilledButton.styleFrom(shape: const StadiumBorder()),
      onPressed: onPressed,
      child: const Text('Choose avatar'),
    );
  }
}
