import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/community/community_provider.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/relay/relay_provider.dart';
import '../../shared/share/share_bestie.dart';
import '../channels/channel.dart';
import '../channels/channels_provider.dart';
import '../channels/dm_channel_labels.dart';

/// Hands the iOS share extension this account's conversations, newest
/// activity first, with the names the channel list shows, so its picker
/// opens without relay queries. Written each time the app goes to the
/// background, the moment before someone could share from another app;
/// nothing is watched or written while the app is in use.
class ShareConversationsPublisher extends HookConsumerWidget {
  const ShareConversationsPublisher({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useEffect(() {
      final listener = AppLifecycleListener(
        onHide: () => unawaited(_publish(ref)),
      );
      return listener.dispose;
    }, const []);
    return const SizedBox.shrink();
  }

  static Future<void> _publish(WidgetRef ref) async {
    final me = ref.read(myPubkeyProvider)?.toLowerCase();
    final community = ref.read(activeCommunityProvider).value?.id;
    final channels = ref.read(channelsProvider).value;
    if (me == null || community == null || channels == null) return;
    final names = ref.read(identityNameSourcesProvider);
    final active = [
      for (final channel in channels)
        if (channel.isMember && !channel.isArchived) channel,
    ];
    int at(Channel channel) =>
        (channel.lastMessageAt ?? channel.createdAt).millisecondsSinceEpoch ~/
        1000;
    active.sort((a, b) => at(b).compareTo(at(a)));
    await publishShareConversations(
      pubkey: me,
      community: community,
      conversations: jsonEncode([
        for (final channel in active)
          {
            'id': channel.id,
            'name': resolveDmChannelDisplayLabel(
              channel,
              currentPubkey: me,
              names: names,
            ),
            'dm': channel.isDm,
            'solo': channel.isSolo,
            'recipients': [
              if (channel.isDm)
                for (final p in channel.participantPubkeys)
                  if (p.toLowerCase() != me) p.toLowerCase(),
            ],
            'at': at(channel),
          },
      ]),
    );
  }
}
