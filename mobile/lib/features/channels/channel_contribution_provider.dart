import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'channel.dart';
import 'channel_management_provider.dart';

/// Current roster contribution permission for presentation and send preflight.
/// Null lets callers use a saved snapshot while offline or loading; the relay
/// authorizes publication.
final channelContributionProvider = Provider.family<bool?, String>((ref, id) {
  final actor = ref.watch(currentPubkeyProvider)?.toLowerCase();
  final status = ref.watch(
    relaySessionProvider.select((session) => session.status),
  );
  if (actor == null || status != SessionStatus.connected) return null;
  final members = ref.watch(channelMembersProvider(id));
  if (!members.hasValue || members.hasError) return null;
  final roster = members.asData?.value;
  if (roster == null) return null;
  final own = roster
      .where((member) => member.pubkey.toLowerCase() == actor)
      .toList();
  return own.length == 1 && channelRoleCanContribute(own.single.role);
});
