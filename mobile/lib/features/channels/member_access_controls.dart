import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'channel_access_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import 'channel_management_provider.dart';

/// Confirmed individual access management, with readback-only recovery after
/// uncertain publication. Owner/self roles are immutable in this surface.
class MemberAccessControls extends HookConsumerWidget {
  const MemberAccessControls({
    super.key,
    required this.channelId,
    required this.target,
    required this.label,
  });
  final String channelId;
  final String target;
  final String label;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = ref.watch(channelAccessProvider.notifier);
    final pending = ref.watch(channelAccessProvider)[channelId];
    final reload = useState(0);
    final request = useMemoized(() => controller.load(channelId), [
      controller,
      channelId,
      reload.value,
    ]);
    final snapshot = useFuture(request);
    final error = useState<String?>(null);
    final busy = useState(false);
    final actor = ref.watch(currentPubkeyProvider);
    final base = snapshot.data;
    final role = base?.role(target);
    final editable =
        base?.canEdit(actor) == true &&
        actor != target &&
        const {'admin', 'member', 'guest', 'bot'}.contains(role);

    final owners =
        !editable &&
            base != null &&
            base.role(actor) != null &&
            !base.data.labels.contains('buzz.solo')
        ? ref.watch(agentOwnersProvider).asData?.value ??
              const <String, String>{}
        : const <String, String>{};
    final ownRemoval =
        base != null &&
        !base.data.isArchived &&
        base.data.channelType != 'dm' &&
        !base.data.labels.contains('buzz.solo') &&
        base.role(actor) != null &&
        owners[target] == actor &&
        role != 'owner' &&
        role != null;
    Future<void> perform({
      String? nextRole,
      bool check = false,
      bool discard = false,
    }) async {
      if (busy.value || base == null) return;
      if (discard) {
        if (await confirmDiscardAccessSave(context) != true ||
            !context.mounted) {
          return;
        }
      } else if (!check) {
        final message = nextRole == null
            ? 'Remove $label from this channel?'
            : 'Set $label to ${memberAccessLabel(nextRole).toLowerCase()}?';
        final confirmed = await showDialog<bool>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: Text(nextRole == null ? 'Remove member?' : 'Change access?'),
            content: Text(message),
            actions: [
              TextButton(
                autofocus: true,
                onPressed: () => Navigator.pop(dialogContext, false),
                child: const Text('Cancel'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(dialogContext, true),
                child: Text(nextRole == null ? 'Remove' : 'Change access'),
              ),
            ],
          ),
        );
        if (!context.mounted || confirmed != true) return;
      }
      busy.value = true;
      error.value = null;
      try {
        if (discard && pending != null) {
          await controller.discardSave(channelId, pending);
        } else if (check) {
          await controller.checkSave(channelId);
        } else {
          await controller.changeMember(
            channelId,
            base,
            target,
            role: nextRole,
          );
        }
        if (context.mounted) reload.value++;
      } catch (reason) {
        if (context.mounted) {
          error.value = reason.toString();
          reload.value++;
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return Material(
      color: Colors.transparent,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (snapshot.connectionState != ConnectionState.done)
              const BuzzLoadingIndicator(
                size: 24,
                semanticLabel: 'Checking member access',
              ),
            if (editable &&
                base?.data.labels.contains('buzz.solo') != true) ...[
              for (final nextRole in ['admin', 'member', 'guest'])
                ListTile(
                  title: Text(memberAccessLabel(nextRole)),
                  selected:
                      nextRole == role ||
                      (nextRole == 'member' && role == 'bot'),
                  enabled:
                      !busy.value &&
                      pending == null &&
                      nextRole != role &&
                      !(nextRole == 'member' && role == 'bot'),
                  onTap: () => perform(nextRole: nextRole),
                ),
            ],
            if (editable || ownRemoval)
              ListTile(
                title: Text(
                  'Remove from channel',
                  style: TextStyle(color: context.colors.error),
                ),
                enabled: !busy.value && pending == null,
                onTap: () => perform(),
              ),
            if (busy.value)
              Semantics(liveRegion: true, child: Text('Saving access…')),
            if (error.value != null || snapshot.hasError)
              Semantics(
                liveRegion: true,
                child: Text(
                  error.value ?? 'Current access could not be confirmed.',
                  style: TextStyle(color: context.colors.error),
                ),
              ),
            if (pending != null) ...[
              TextButton(
                onPressed: busy.value ? null : () => perform(check: true),
                child: const Text('Check save status'),
              ),
              TextButton(
                onPressed: busy.value ? null : () => perform(discard: true),
                child: const Text('Discard and reload'),
              ),
            ] else if (snapshot.hasError || error.value != null)
              TextButton(
                onPressed: () {
                  error.value = null;
                  reload.value++;
                },
                child: const Text('Reload access'),
              ),
          ],
        ),
      ),
    );
  }
}

/// Confirms abandoning local recovery without promising to cancel publication.
Future<bool?> confirmDiscardAccessSave(
  BuildContext context,
) => showDialog<bool>(
  context: context,
  builder: (dialogContext) => AlertDialog(
    title: const Text('Discard this save check?'),
    content: const Text(
      'The earlier request may still arrive. Reload current access and review it before making another change. Nothing will be resent.',
    ),
    actions: [
      TextButton(
        autofocus: true,
        onPressed: () => Navigator.pop(dialogContext, false),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: () => Navigator.pop(dialogContext, true),
        child: const Text('Discard and reload'),
      ),
    ],
  ),
);

/// Product access labels keep protocol role names out of permission controls.
String memberAccessLabel(String role) => switch (role) {
  'owner' => 'Owner',
  'admin' => 'Admin',
  'guest' => 'Viewing only',
  'member' || 'bot' => 'Participating',
  _ => 'Access unknown',
};
