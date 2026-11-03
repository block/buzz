import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/nostr_models.dart' show membersFromEvent;
import '../../shared/theme/theme.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'channel.dart';
import 'channel_access_provider.dart';
import 'channel_management_provider.dart';
import 'member_access_controls.dart' show confirmDiscardAccessSave;

/// Mobile access inspector/editor, shared by ordinary channels and Solo chats.
Future<void> showChannelAccessSheet(
  BuildContext context,
  Channel channel, {
  ValueChanged<ChannelDetails>? onUpdated,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  title: 'General access',
  showDragHandle: true,
  isScrollControlled: true,
  builder: (_) => ChannelAccessSheet(channel: channel, onUpdated: onUpdated),
);

class ChannelAccessSheet extends HookConsumerWidget {
  const ChannelAccessSheet({super.key, required this.channel, this.onUpdated});
  final Channel channel;
  final ValueChanged<ChannelDetails>? onUpdated;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = ref.watch(channelAccessProvider.notifier);
    final attempt = ref.watch(channelAccessProvider)[channel.id];
    final reload = useState(0);
    final request = useMemoized(() => controller.load(channel.id), [
      controller,
      channel.id,
      reload.value,
    ]);
    final snapshot = useFuture(request);
    final busy = useState(false);
    final error = useState<String?>(null);
    final status = useState('');
    final actor = ref.watch(currentPubkeyProvider);
    final base = snapshot.data;
    final data = base?.data;
    final solo = data?.labels.contains('buzz.solo') ?? channel.isSolo;
    final place = solo ? 'chat' : 'channel';
    final visibility = data?.visibility ?? channel.visibility;
    final joinRole = data?.joinRole ?? channel.joinRole;
    final access = visibility == 'private'
        ? 'private'
        : joinRole == 'guest'
        ? 'view-only'
        : 'public';
    final accessLabel = access == 'private'
        ? 'Private'
        : access == 'view-only'
        ? 'View only'
        : solo
        ? 'Public (existing)'
        : 'Public';
    final confirmed =
        snapshot.connectionState == ConnectionState.done &&
        !snapshot.hasError &&
        base != null;
    final editable = confirmed && base.canEdit(actor);
    final choicesEnabled = editable && !busy.value && attempt == null;

    Future<void> perform(
      Future<ChannelAccessSnapshot> Function() action, {
      String success = 'Access saved.',
    }) async {
      if (busy.value) return;
      busy.value = true;
      error.value = null;
      status.value = 'Saving access…';
      try {
        final saved = await action();
        onUpdated?.call(channelDetailsFromEvent(saved.metadata));
        if (!context.mounted) return;
        status.value = success;
        reload.value++;
      } catch (reason) {
        if (context.mounted) {
          error.value = reason.toString();
          status.value = '';
          reload.value++;
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    Future<void> choose(String choice) async {
      if (!editable || busy.value || attempt != null || choice == access) {
        return;
      }
      String message;
      if (solo && choice == 'private') {
        message = 'Only chat members will have access.';
        if (access == 'view-only' &&
            membersFromEvent(
              base.roster,
            ).any((member) => member.role == 'guest')) {
          message +=
              ' People who joined while it was View only are members and keep reading until you remove them in People.';
        }
      } else if (solo && choice == 'view-only') {
        message =
            "Everyone in this community will be able to read this chat's full history and Canvas.";
        if (access == 'public') {
          message += ' People already in the chat keep their access.';
        }
      } else {
        message = choice == 'private'
            ? 'Only people already added to this $place will be able to read. Existing members keep their access.'
            : '${choice == 'view-only' ? 'Anyone in the community will be able to read the full history of this $place.' : 'Anyone in the community will be able to read the full history, join and participate.'} Existing members keep their individual access.';
      }
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (dialogContext) => AlertDialog(
          title: const Text('Change general access?'),
          content: Text(message),
          actions: [
            TextButton(
              autofocus: true,
              onPressed: () => Navigator.pop(dialogContext, false),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(dialogContext, true),
              child: const Text('Change access'),
            ),
          ],
        ),
      );
      if (confirmed != true || !context.mounted) return;
      await perform(() => controller.setAccess(channel.id, base, choice));
    }

    return Material(
      color: Colors.transparent,
      child: SafeArea(
        child: SingleChildScrollView(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.md,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (!confirmed || (editable && solo && access == 'public'))
                  Text(
                    'Current access: $accessLabel',
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.onSurfaceVariant,
                    ),
                  ),
                if (snapshot.connectionState != ConnectionState.done)
                  const Padding(
                    padding: EdgeInsets.all(Grid.xs),
                    child: BuzzLoadingIndicator(
                      size: 24,
                      semanticLabel: 'Checking access',
                    ),
                  ),
                if (confirmed) ...[
                  if (!solo || (!editable && access == 'public'))
                    ListTile(
                      leading: const Icon(BuzzIcons.users),
                      title: Text(solo ? 'Public (existing)' : 'Public'),
                      subtitle: const Text(
                        'Anyone in the community can participate.',
                      ),
                      selected: access == 'public',
                      selectedColor: context.colors.primary,
                      trailing: access == 'public'
                          ? const Icon(BuzzIcons.check)
                          : null,
                      enabled: choicesEnabled,
                      onTap: () => choose('public'),
                    ),
                  ListTile(
                    leading: const Icon(BuzzIcons.shieldCheck),
                    title: const Text('View only'),
                    subtitle: const Text('Anyone in the community can read.'),
                    selected: access == 'view-only',
                    selectedColor: context.colors.primary,
                    trailing: access == 'view-only'
                        ? const Icon(BuzzIcons.check)
                        : null,
                    enabled: choicesEnabled,
                    onTap: () => choose('view-only'),
                  ),
                  ListTile(
                    leading: const Icon(BuzzIcons.lock),
                    title: const Text('Private'),
                    subtitle: const Text('Only people you add can read.'),
                    selected: access == 'private',
                    selectedColor: context.colors.primary,
                    trailing: access == 'private'
                        ? const Icon(BuzzIcons.check)
                        : null,
                    enabled: choicesEnabled,
                    onTap: () => choose('private'),
                  ),
                ],
                if (confirmed && !editable)
                  Text(
                    data?.isArchived == true
                        ? 'This $place is archived.'
                        : channelRoleCanContribute(base.role(actor))
                        ? 'Only owners and admins can change access.'
                        : base.role(actor) == null &&
                              visibility == 'open' &&
                              joinRole == 'member' &&
                              !solo
                        ? 'Join this $place to participate.'
                        : 'Viewing only. Ask an owner or admin to participate in this $place.',
                  ),
                if (status.value.isNotEmpty)
                  Semantics(liveRegion: true, child: Text(status.value)),
                if (snapshot.hasError || error.value != null)
                  Semantics(
                    liveRegion: true,
                    child: Text(
                      error.value ?? 'Current access could not be confirmed.',
                      style: TextStyle(color: context.colors.error),
                    ),
                  ),
                if (attempt != null) ...[
                  TextButton(
                    onPressed: busy.value
                        ? null
                        : () => perform(() => controller.checkSave(channel.id)),
                    child: const Text('Check save status'),
                  ),
                  TextButton(
                    onPressed: busy.value
                        ? null
                        : () async {
                            if (await confirmDiscardAccessSave(context) !=
                                    true ||
                                !context.mounted) {
                              return;
                            }
                            await perform(
                              () => controller.discardSave(channel.id, attempt),
                              success:
                                  'Access reloaded. Review the current setting.',
                            );
                          },
                    child: const Text('Discard and reload'),
                  ),
                ] else if (snapshot.hasError || error.value != null)
                  TextButton(
                    onPressed: busy.value
                        ? null
                        : () {
                            error.value = null;
                            reload.value++;
                          },
                    child: const Text('Reload access'),
                  ),
                const SizedBox(height: Grid.xs),
                TextButton(
                  onPressed: () => Navigator.pop(context),
                  child: const Text('Done'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
