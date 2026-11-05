import 'access_error_message.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_chat_share.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'channel.dart';
import 'channel_access_provider.dart';
import 'channels_provider.dart';

/// Sharing opens community reading and posts one link without enrolling anyone.
Future<void> showSoloShareSheet(BuildContext context, Channel channel) =>
    showBuzzModalBottomSheet<void>(
      context: context,
      title: 'Share chat',
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => SoloShareSheet(channel: channel),
    );

/// Confirms community reading before posting a link; individual roles stay intact.
class SoloShareSheet extends HookConsumerWidget {
  const SoloShareSheet({super.key, required this.channel});
  final Channel channel;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final sync = ref.watch(meSyncProvider);
    final controller = ref.watch(channelAccessProvider.notifier);
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
        final previous = pending.value;
        final destinationId = previous?.destination ?? destination?.id;
        if (destinationId == null) return;
        final unfinished = ref.read(channelAccessProvider)[channel.id];
        if (unfinished != null) {
          if (previous == null ||
              unfinished.target != null ||
              unfinished.visibility != 'open' ||
              unfinished.joinRole != 'guest' ||
              unfinished.labels != null) {
            throw StateError(
              'Finish the existing General access change first.',
            );
          }
          // Readback only: never replay an uncertain access write.
          await controller.checkSave(channel.id);
          check();
        }
        final source = await controller.load(channel.id);
        await controller.requireContribution(destinationId);
        check();
        void requireSource(ChannelAccessSnapshot snapshot) {
          if (!snapshot.data.labels.contains('buzz.solo') ||
              snapshot.data.channelType != 'stream' ||
              !channelRoleCanContribute(snapshot.role(sync.pubkey)) ||
              snapshot.data.isArchived) {
            throw StateError('This chat is no longer available for sharing.');
          }
          if (snapshot.data.visibility == 'private' &&
              (!snapshot.canEdit(sync.pubkey) ||
                  snapshot.data.creator != sync.pubkey)) {
            throw StateError(
              'Only the chat creator with owner or admin access can open this chat for community reading.',
            );
          }
        }

        requireSource(source);
        final destinationName = previous?.destinationName ?? destination!.name;
        if (!context.mounted) return;
        if (source.data.visibility == 'private' ||
            source.data.joinRole == 'guest') {
          final confirmed = await showBuzzDialog<bool>(
            context: context,
            builder: (dialogContext) => AlertDialog(
              title: Text('Share in #$destinationName?'),
              content: Text(
                '${source.data.visibility == 'private' ? 'This chat will change to View only. ' : ''}'
                'Everyone in this community can read its full history, Canvas, workspace folders, and agent instructions. '
                'One link will be posted in the channel. No one is added to the chat, and existing members keep their roles.',
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
        }
        final selected =
            previous ??
            MeChatShare.create(
              sync,
              source: channel.id,
              name: source.data.name,
              destination: destinationId,
              destinationName: destinationName,
              forum: destination!.isForum,
            );
        pending.value = selected;
        await selected.complete(
          sync,
          prepare: (checkPending) async {
            void active() {
              check();
              checkPending();
            }

            active();
            await controller.requireContribution(selected.destination);
            active();
            final current = await controller.load(selected.source);
            active();
            requireSource(current);
            if (current.data.visibility == 'private') {
              if (source.data.visibility != 'private' ||
                  current.metadata.id != source.metadata.id) {
                throw StateError('Chat access changed. Review sharing again.');
              }
              await controller.setAccess(
                selected.source,
                current,
                'view-only',
                beforeDispatch: active,
              );
              active();
            }
            // Already-open policies, including Public, remain untouched.
            await controller.requireContribution(selected.destination);
            active();
          },
          send: (message) async {
            check();
            await sync.session.publish(message);
          },
        );
        if (context.mounted) Navigator.pop(context);
      } catch (reason) {
        if (context.mounted) {
          pending.value = MeChatShare.pending(sync, channel.id);
          error.value = accessErrorMessage(reason);
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    Future<void> cancelShare() async {
      final reviewed = pending.value;
      if (busy.value || reviewed == null) return;
      busy.value = true;
      try {
        final confirmed = await showBuzzDialog<bool>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: const Text('Cancel this share?'),
            content: const Text(
              'Any saved General access change and a possibly-posted link may remain. This only forgets the pending share; nothing will be resent and access will not change.',
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(dialogContext, false),
                child: const Text('Keep share'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(dialogContext, true),
                child: const Text('Cancel share'),
              ),
            ],
          ),
        );
        if (confirmed != true ||
            !context.mounted ||
            !identical(pending.value, reviewed)) {
          return;
        }
        await reviewed.discard(sync);
        if (context.mounted) {
          pending.value = null;
          error.value = null;
        }
      } catch (reason) {
        if (context.mounted) error.value = accessErrorMessage(reason);
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
                  'Finish sharing in #${intent.destinationName}. Any saved General access change stays in place. Uncertain retries reuse the link; a rejected link is refreshed before the next retry.',
                ),
                FilledButton(
                  onPressed: busy.value ? null : () => share(),
                  child: const Text('Retry share'),
                ),
                TextButton(
                  onPressed: busy.value ? null : cancelShare,
                  child: const Text('Cancel share'),
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
