import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/agent_avatar_catalog.dart';
import '../../shared/agents/owned_agent_directory.dart';
import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/feedback_snackbar.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/sheet_action_section.dart';
import 'agent_avatar_field.dart';
import 'agent_avatar_picker.dart';
import 'create_agent_page.dart';

/// Edit name and avatar, for the signed-in owner's active remote agents only.
class EditAgentProfileAction extends HookConsumerWidget {
  const EditAgentProfileAction({
    required this.controller,
    required this.pubkey,
    super.key,
  });

  final RemoteAgentController controller;
  final String pubkey;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final community = ref.watch(activeCommunityProvider).asData?.value;
    // Wait for the profile the sheet loads, so the editor starts from the
    // agent's real avatar.
    final profile = ref.watch(
      userCacheProvider.select((cache) => cache[pubkey]),
    );
    final agent = controller.agents
        .where((row) => row['agent_pubkey'] == pubkey)
        .firstOrNull;
    final name = agent?['agent_name'];
    if (profile == null ||
        community == null ||
        community.pubkey == null ||
        community.pubkey != controller.owner ||
        !controller.activeAgents.contains(pubkey) ||
        name is! String) {
      return const SizedBox.shrink();
    }
    return Padding(
      padding: const EdgeInsets.only(bottom: Grid.sm),
      child: SheetActionSection(
        children: [
          AppListRow(
            onTap: controller.busy
                ? null
                : () => Navigator.of(context).push(
                    MaterialPageRoute<void>(
                      fullscreenDialog: true,
                      builder: (_) => EditAgentProfilePage(
                        controller: controller,
                        community: community,
                        pubkey: pubkey,
                        name: name,
                        picture: profile.avatarUrl,
                      ),
                    ),
                  ),
            icon: BuzzIcons.pencil,
            title: 'Edit name and avatar',
          ),
        ],
      ),
    );
  }
}

/// Renames a remote agent and replaces its avatar through Builderlab.
class EditAgentProfilePage extends HookConsumerWidget {
  const EditAgentProfilePage({
    required this.controller,
    required this.community,
    required this.pubkey,
    required this.name,
    required this.picture,
    super.key,
  });
  final RemoteAgentController controller;
  final Community community;
  final String pubkey;
  final String name;

  /// The agent's current avatar, if it has one.
  final String? picture;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final field = useTextEditingController(text: name);
    useListenable(field);
    final initial = useMemoized(
      () => AgentAvatarSelection.url(picture ?? agentAvatarCatalog.first.url),
    );
    final avatar = useState(initial);
    final heroTag = useMemoized(Object.new);
    final editingAvatar = useState(false);
    final preparing = useState(false);
    final error = useState<String?>(null);
    final busy = controller.busy || preparing.value;
    final changed =
        field.text.trim() != name || !identical(avatar.value, initial);
    final canSave = !busy && changed && field.text.trim().isNotEmpty;

    Future<void> save() async {
      error.value = null;
      // Check the name before uploading anything.
      if (!validAgentName(field.text.trim())) {
        error.value = invalidAgentName;
        return;
      }
      preparing.value = true;
      try {
        final subject = controller.account.account?.subject;
        bool current() =>
            context.mounted &&
            controller.account.account?.subject == subject &&
            ref.read(activeCommunityProvider).asData?.value?.id == community.id;
        final String? uploaded;
        try {
          // An unchanged avatar is kept by Builderlab, so send none.
          uploaded = identical(avatar.value, initial)
              ? null
              : await avatar.value.prepare(
                  ref.read(mediaUploadServiceProvider),
                );
        } catch (_) {
          if (context.mounted) {
            error.value = 'Could not upload the avatar. Try again.';
          }
          return;
        }
        if (!current()) {
          if (context.mounted) {
            error.value = 'The account or community changed. Reopen Agents.';
          }
          return;
        }
        final saved = await controller.updateProfile(
          community: community,
          pubkey: pubkey,
          name: field.text,
          pictureUrl: uploaded,
          isCurrent: current,
          fetch: ref.read(relaySessionProvider.notifier).fetchHistory,
        );
        if (!context.mounted) return;
        if (!saved) {
          final failure = controller.failure;
          error.value = failure.isEmpty
              ? 'Could not update the agent. Try again.'
              : failure;
          return;
        }
        ref.read(userCacheProvider.notifier).refresh([pubkey]);
        ref.invalidate(agentDirectoryProvider);
        ref.invalidate(ownedAgentDirectoryProvider);
        ScaffoldMessenger.of(context).showFeedbackSnackBar(
          const SnackBar(content: Text('Agent updated')),
          kind: ToastKind.success,
        );
        Navigator.of(context).pop();
      } finally {
        if (context.mounted) preparing.value = false;
      }
    }

    return PopScope(
      canPop: !busy,
      child: FrostedScaffold(
        appBar: FrostedAppBar(
          // Keep the full-screen UIKit navigation layer out of the Hero's
          // overlay until the editor's reverse transition has completed.
          nativeViewSuppressed: editingAvatar,
          nativeTitle: 'Edit agent',
          title: const Text('Edit agent'),
          automaticallyImplyLeading: false,
          nativeLeading: IosNavigationAction(
            label: 'Cancel',
            symbol: 'xmark',
            onPressed: busy ? null : () => Navigator.pop(context),
          ),
          nativeActions: [
            IosNavigationAction(
              label: 'Save',
              symbol: 'checkmark',
              onPressed: canSave ? save : null,
            ),
          ],
          leading: IconButton(
            tooltip: 'Cancel',
            onPressed: busy ? null : () => Navigator.pop(context),
            icon: const Icon(Icons.close),
          ),
          actions: [
            IconButton(
              tooltip: 'Save',
              onPressed: canSave ? save : null,
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
                AgentAvatarField(
                  selection: avatar.value,
                  heroTag: heroTag,
                  editing: editingAvatar,
                  enabled: !busy,
                  onChanged: (value) => avatar.value = value,
                ),
                const SizedBox(height: Grid.md),
                SheetActionSection(
                  label: 'Name',
                  children: [
                    Semantics(
                      label: 'Name',
                      child: TextField(
                        controller: field,
                        enabled: !busy,
                        textCapitalization: TextCapitalization.words,
                        decoration: agentFieldDecoration(context),
                      ),
                    ),
                  ],
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
              ],
            ),
          ),
        ),
      ),
    );
  }
}
