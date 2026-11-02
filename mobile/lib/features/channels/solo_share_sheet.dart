import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/crypto/nip_oa.dart';
import '../../shared/me/me_chat_share.dart';
import '../../shared/me/me_archived_agents.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'channel.dart';
import 'channel_access_provider.dart';
import 'channels_provider.dart';

/// Sharing grants access and posts one chat link; it never converts the chat.
Future<void> showSoloShareSheet(BuildContext context, Channel channel) =>
    showBuzzModalBottomSheet<void>(
      context: context,
      title: 'Share chat',
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => SoloShareSheet(channel: channel),
    );

/// Reviews a fixed channel audience before granting access and sharing a link.
class SoloShareSheet extends HookConsumerWidget {
  const SoloShareSheet({super.key, required this.channel});
  final Channel channel;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final controller = ref.watch(channelAccessProvider.notifier);
    final container = ProviderScope.containerOf(context, listen: false);
    final pending = useState(MeChatShare.pending(sync, channel.id));
    final query = useTextEditingController();
    useListenable(query);
    final busy = useState(false);
    final error = useState<String?>(null);
    final channels = ref.watch(channelsProvider);
    final choices = (channels.asData?.value ?? const <Channel>[])
        .where(
          (candidate) =>
              candidate.id != channel.id &&
              !candidate.isSolo &&
              candidate.canContribute &&
              (candidate.isStream || candidate.isForum) &&
              query.text.trim().isNotEmpty &&
              candidate.name.toLowerCase().contains(
                query.text.trim().toLowerCase(),
              ),
        )
        .toList();

    void check() {
      sync.check();
      if (!context.mounted) {
        throw StateError('Share paused. Open sharing again to finish.');
      }
    }

    Future<void> share({Channel? destination}) async {
      if (busy.value) return;
      busy.value = true;
      error.value = null;
      try {
        var intent = pending.value;
        if (intent == null) {
          if (destination == null) return;
          final snapshots = await Future.wait([
            controller.load(channel.id),
            controller.load(destination.id),
          ]);
          check();
          final source = snapshots[0];
          final target = snapshots[1];
          if (!source.data.labels.contains('buzz.solo') ||
              !channelRoleCanContribute(source.role(sync.pubkey)) ||
              source.data.isArchived) {
            throw StateError('This chat is no longer available for sharing.');
          }
          await controller.requireContribution(destination.id);
          final archived = await container.read(
            meArchivedAgentsProvider.future,
          );
          check();
          final recipients = membersFromEvent(target.roster)
              .where(
                (member) =>
                    !source.contains(member.pubkey) &&
                    !archived.contains(member.pubkey),
              )
              .toList();
          if (recipients.length > 100) {
            throw StateError(
              'Share with at most 100 people or agents at a time.',
            );
          }
          final profiles = recipients.isEmpty
              ? const <NostrEvent>[]
              : await sync.session.fetchHistory(
                  NostrFilters.profilesBatch(
                    recipients.map((member) => member.pubkey).toList(),
                  ),
                );
          check();
          final agents = {
            for (final event in profiles)
              if (verifiedProfileOaOwnerPubkey(event) != null) event.pubkey,
          };
          if (!context.mounted) return;
          final confirmed = await showDialog<bool>(
            context: context,
            builder: (dialogContext) => AlertDialog(
              title: Text('Share in #${destination.name}?'),
              content: Text(
                '${recipients.length} people or agents will receive access to this chat. One link will be posted in the channel. Future joins and departures will not change access to this chat.',
              ),
              actions: [
                TextButton(
                  autofocus: true,
                  onPressed: () => Navigator.pop(dialogContext, false),
                  child: const Text('Cancel'),
                ),
                FilledButton(
                  onPressed: () => Navigator.pop(dialogContext, true),
                  child: const Text('Share'),
                ),
              ],
            ),
          );
          if (confirmed != true || !context.mounted) return;
          intent = MeChatShare.create(
            sync,
            source: channel.id,
            name: source.data.name,
            destination: destination.id,
            destinationName: destination.name,
            forum: destination.isForum,
            audience: {
              for (final member in recipients)
                member.pubkey:
                    member.role == 'bot' || agents.contains(member.pubkey)
                    ? 'bot'
                    : 'member',
            },
          );
          pending.value = intent;
        }
        final selected = intent;
        await selected.complete(
          sync,
          ensureRecipient: (recipient, role) async {
            check();
            final destinationNow = await controller.load(selected.destination);
            check();
            // Frozen consent may shrink when people leave; it never grows.
            if (!destinationNow.contains(recipient)) return;
            final sourceNow = await controller.load(selected.source);
            if (!sourceNow.data.labels.contains('buzz.solo')) {
              throw StateError('The chat was converted. Review sharing again.');
            }
            await controller.addMember(
              selected.source,
              recipient,
              role,
              beforeDispatch: check,
            );
          },
          send: (message) async {
            check();
            await controller.requireContribution(selected.destination);
            check();
            await sync.session.publish(message);
          },
        );
        if (context.mounted) Navigator.pop(context);
      } catch (reason) {
        if (context.mounted) error.value = reason.toString();
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          Grid.md,
        ),
        child: SizedBox(
          height: MediaQuery.sizeOf(context).height * .65,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (pending.value case final intent?) ...[
                Text(
                  'Finish sharing in #${intent.destinationName}. Confirmed access stays in place; Retry uses the same link message.',
                ),
                FilledButton(
                  onPressed: busy.value ? null : () => share(),
                  child: const Text('Retry share'),
                ),
              ] else ...[
                TextField(
                  controller: query,
                  decoration: const InputDecoration(
                    labelText: 'Search channels',
                  ),
                ),
                Expanded(
                  child: channels.when(
                    data: (_) => ListView(
                      children: [
                        for (final destination in choices)
                          ListTile(
                            title: Text('#${destination.name}'),
                            enabled: !busy.value,
                            onTap: () => share(destination: destination),
                          ),
                      ],
                    ),
                    error: (_, _) =>
                        const Text('Channels could not be loaded.'),
                    loading: () => const Center(
                      child: BuzzLoadingIndicator(
                        size: 24,
                        semanticLabel: 'Loading channels',
                      ),
                    ),
                  ),
                ),
              ],
              if (busy.value)
                const Padding(
                  padding: EdgeInsets.all(Grid.xxs),
                  child: BuzzLoadingIndicator(
                    size: 24,
                    semanticLabel: 'Sharing chat',
                  ),
                ),
              if (error.value != null)
                Semantics(
                  liveRegion: true,
                  child: Text(
                    error.value!,
                    style: TextStyle(color: context.colors.error),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
