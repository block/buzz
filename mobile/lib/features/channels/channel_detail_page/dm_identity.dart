part of '../channel_detail_page.dart';

Future<bool> _preloadMembers(
  WidgetRef ref,
  String channelId,
  List<String> participantPubkeys, {
  required bool refreshDmParticipants,
}) async {
  // Capture references before async gap to avoid using disposed ref.
  final notifier = ref.read(userCacheProvider.notifier);
  try {
    final members = await ref.read(channelMembersProvider(channelId).future);
    await notifier.preload(members.map((member) => member.pubkey).toList());
    if (refreshDmParticipants) {
      return await notifier.refresh(participantPubkeys);
    }
    return true;
  } catch (_) {
    // Identity remains unresolved, so agent-only actions stay hidden.
    return false;
  }
}

Future<void Function()> _subscribeToDmIdentityUpdates(
  WidgetRef ref,
  List<String> participantPubkeys, {
  required ValueChanged<bool> onReadyChanged,
  required ValueChanged<Set<String>> onAgentPubkeysChanged,
  required VoidCallback onFailure,
}) async {
  final session = ref.read(relaySessionProvider.notifier);
  var subscriptionStatus = RelaySubscriptionStatus.retrying;
  var directLookupComplete = false;
  final agentPubkeys = <String>{};

  void publishAgentPubkeys() {
    onAgentPubkeysChanged(Set.unmodifiable(agentPubkeys));
  }

  void handleEvent(NostrEvent event) {
    if (event.kind == 0) {
      try {
        ref.read(userCacheProvider.notifier).cacheProfileEvent(event);
      } catch (error) {
        debugPrint('[DmIdentity] invalid live profile: $error');
        onFailure();
      }
    } else if (event.kind == 10100) {
      agentPubkeys.add(event.pubkey.toLowerCase());
      publishAgentPubkeys();
      ref.invalidate(agentDirectoryProvider);
      ref.invalidate(agentOwnersProvider);
    }
  }

  final unsubscribe = await session.subscribeWithStatus(
    NostrFilter(
      kinds: const [0, 10100],
      authors: participantPubkeys,
      limit: 100,
    ).copyWithSince(DateTime.now().millisecondsSinceEpoch ~/ 1000 - 5),
    handleEvent,
    onClosed: (_) => onFailure(),
    onStatusChanged: (status) {
      subscriptionStatus = status;
      if (status == RelaySubscriptionStatus.retrying) {
        onReadyChanged(false);
      } else if (directLookupComplete) {
        onReadyChanged(true);
      }
    },
  );

  try {
    final profiles = await session.fetchHistory(
      NostrFilter(
        kinds: const [10100],
        authors: participantPubkeys,
        limit: participantPubkeys.length,
      ),
    );
    for (final profile in profiles) {
      if (profile.kind == 10100) {
        agentPubkeys.add(profile.pubkey.toLowerCase());
      }
    }
    publishAgentPubkeys();
    directLookupComplete = true;
    onReadyChanged(subscriptionStatus == RelaySubscriptionStatus.ready);
    return unsubscribe;
  } catch (_) {
    unsubscribe();
    rethrow;
  }
}

/// The time that opening [channel] reads it through, or null when opening it
/// reads nothing. Forums read through their last activity. DMs read through
/// the newest loaded message, including replies, or the last activity before
/// messages load.
int? _openReadTimestamp({
  required Channel channel,
  required AsyncValue<List<NostrEvent>> messagesState,
}) {
  if (channel.isForum) return dateTimeToUnixSeconds(channel.lastMessageAt);
  if (!channel.isDm) return null;
  var latest = 0;
  for (final event in messagesState.value ?? const <NostrEvent>[]) {
    if (event.createdAt > latest) latest = event.createdAt;
  }
  return latest > 0 ? latest : dateTimeToUnixSeconds(channel.lastMessageAt);
}

bool _isOneToOneAgentDm(Channel channel, Set<String> agentPubkeys) {
  final participants = channel.participantPubkeys
      .map((pubkey) => pubkey.trim().toLowerCase())
      .where((pubkey) => pubkey.isNotEmpty)
      .toSet();
  return channel.isDm &&
      participants.length == 2 &&
      participants.any(agentPubkeys.contains);
}
