import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channel.dart';
import 'channel_management_provider.dart';

/// Current roster contribution permission for presentation and send preflight.
/// Null lets callers use a saved snapshot before any roster has loaded; the relay
/// authorizes publication.
final channelContributionProvider = Provider.family<bool?, String>((ref, id) {
  final actor = ref.watch(currentPubkeyProvider)?.toLowerCase();
  if (actor == null) return null;
  final roster = watchLatestChannelMembers(ref, id);
  if (roster == null) return null;
  final own = roster
      .where((member) => member.pubkey.toLowerCase() == actor)
      .toList();
  return own.length == 1 && channelRoleCanContribute(own.single.role);
});
