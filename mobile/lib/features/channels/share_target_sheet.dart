import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/composer/composer_seed_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/share_inbox/share_inbox_provider.dart';
import '../../shared/share_inbox/shared_payload.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/modal_presentation.dart';
import '../activity/compose_drafts_provider.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'dm_channel_labels.dart';

/// Lets the user pick where a share-sheet payload should go.
///
/// Returns the chosen channel after the payload has been handed to that
/// channel's composer (text into the persisted draft, files into the one-shot
/// composer seed) and released from the inbox. Returns null when dismissed;
/// the payload is discarded so it does not resurface on the next resume.
Future<Channel?> showShareTargetSheet(
  BuildContext context,
  SharedPayload payload,
) => showBuzzModalBottomSheet<Channel>(
  context: context,
  isScrollControlled: true,
  showDragHandle: true,
  title: 'Share to',
  constraints: BoxConstraints(
    maxWidth: 640,
    maxHeight: MediaQuery.sizeOf(context).height * 0.8,
  ),
  builder: (_) => ShareTargetSheet(payload: payload),
);

class ShareTargetSheet extends HookConsumerWidget {
  final SharedPayload payload;

  const ShareTargetSheet({super.key, required this.payload});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final query = useState('');
    final channelsAsync = ref.watch(channelsProvider);
    final myPubkey = ref.watch(myPubkeyProvider);

    return channelsAsync.when(
      loading: () => const Padding(
        padding: EdgeInsets.all(Grid.xl),
        child: Center(child: BuzzLoadingIndicator()),
      ),
      error: (_, _) => Padding(
        padding: const EdgeInsets.all(Grid.xl),
        child: Text(
          'Channels are unavailable right now',
          style: context.textTheme.bodyMedium,
          textAlign: TextAlign.center,
        ),
      ),
      data: (channels) {
        final candidates = _destinations(channels, query.value, myPubkey);
        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(Grid.md, 0, Grid.md, Grid.sm),
              child: _SharePreview(payload: payload),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Grid.md),
              child: TextField(
                autofocus: false,
                onChanged: (value) => query.value = value,
                decoration: const InputDecoration(
                  prefixIcon: Icon(BuzzIcons.search, size: 18),
                  hintText: 'Search channels',
                ),
              ),
            ),
            const SizedBox(height: Grid.sm),
            Flexible(
              child: candidates.isEmpty
                  ? Padding(
                      padding: const EdgeInsets.all(Grid.xl),
                      child: Text(
                        'No matching channels',
                        style: context.textTheme.bodyMedium,
                      ),
                    )
                  : ListView.builder(
                      shrinkWrap: true,
                      itemCount: candidates.length,
                      itemBuilder: (context, index) {
                        final channel = candidates[index];
                        // The sheet surface is a ColoredBox; give each row its
                        // own Material so the tap highlight stays visible.
                        return Material(
                          type: MaterialType.transparency,
                          child: ListTile(
                            key: ValueKey('share-target-${channel.id}'),
                            leading: Icon(channelIcon(channel), size: 20),
                            title: Text(
                              resolveDmChannelDisplayLabel(
                                channel,
                                currentPubkey: myPubkey,
                              ),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                            ),
                            onTap: () =>
                                unawaited(_select(context, ref, channel)),
                          ),
                        );
                      },
                    ),
            ),
            SizedBox(height: MediaQuery.paddingOf(context).bottom + Grid.md),
          ],
        );
      },
    );
  }

  Future<void> _select(
    BuildContext context,
    WidgetRef ref,
    Channel channel,
  ) async {
    seedComposerWithSharedPayload(ref, channel.id, payload);
    await ref.read(pendingSharedPayloadProvider.notifier).consume(payload.id);
    if (!context.mounted) return;
    Navigator.of(context).pop(channel);
  }
}

/// Hands a share payload to the composer for [channelId]: text is appended to
/// the persisted draft, files are parked in the one-shot composer seed.
void seedComposerWithSharedPayload(
  WidgetRef ref,
  String channelId,
  SharedPayload payload,
) {
  final text = payload.composedText;
  if (text.isNotEmpty) {
    final drafts = ref.read(composeDraftsProvider.notifier);
    final key = composeDraftKey(channelId);
    final existing = drafts.textFor(key)?.trim();
    drafts.save(
      key: key,
      channelId: channelId,
      text: existing == null || existing.isEmpty ? text : '$existing\n$text',
    );
  }
  final files = payload.files;
  if (files.isNotEmpty) {
    ref.read(composerSeedProvider.notifier).plant(channelId, [
      for (final file in files)
        ComposerSeedFile(
          path: file.path!,
          name: file.name,
          mimeType: file.mimeType,
        ),
    ]);
  }
}

List<Channel> _destinations(
  List<Channel> channels,
  String query,
  String? myPubkey,
) {
  final normalized = query.trim().toLowerCase();
  final candidates = [
    for (final channel in channels)
      if (channel.isMember && !channel.isArchived)
        if (normalized.isEmpty ||
            resolveDmChannelDisplayLabel(
              channel,
              currentPubkey: myPubkey,
            ).toLowerCase().contains(normalized))
          channel,
  ];
  candidates.sort((a, b) {
    final aTime = a.lastMessageAt?.millisecondsSinceEpoch ?? 0;
    final bTime = b.lastMessageAt?.millisecondsSinceEpoch ?? 0;
    return bTime.compareTo(aTime);
  });
  return candidates;
}

class _SharePreview extends StatelessWidget {
  final SharedPayload payload;

  const _SharePreview({required this.payload});

  @override
  Widget build(BuildContext context) {
    final files = payload.files;
    final text = payload.composedText;
    final summary = [
      if (files.isNotEmpty)
        files.length == 1
            ? (files.single.name ?? '1 file')
            : '${files.length} files',
      if (text.isNotEmpty) text,
    ].join(' · ');
    return Semantics(
      label: 'Sharing $summary',
      excludeSemantics: true,
      child: Row(
        children: [
          Icon(
            files.isNotEmpty ? BuzzIcons.file : BuzzIcons.link,
            size: 16,
            color: context.colors.onSurfaceVariant,
          ),
          const SizedBox(width: Grid.xs),
          Expanded(
            child: Text(
              summary,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
        ],
      ),
    );
  }
}
