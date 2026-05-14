import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import 'feed_item.dart';

/// Builds the home activity feed by issuing three parallel REQs over the relay
/// websocket: mentions of me on user-visible channel kinds, approval kinds
/// addressed to me, and NIP-90 agent job events addressed to me.
class ActivityNotifier extends AsyncNotifier<HomeFeedResponse> {
  @override
  Future<HomeFeedResponse> build() {
    ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);

    if (sessionState.status != SessionStatus.connected) {
      if (state.value case final cached?) {
        return Future.value(cached);
      }
      return Completer<HomeFeedResponse>().future;
    }

    return _fetch();
  }

  Future<HomeFeedResponse> _fetch() async {
    final myPk = ref.read(myPubkeyProvider);
    if (myPk == null) {
      return HomeFeedResponse(
        mentions: const [],
        needsAction: const [],
        activity: const [],
        agentActivity: const [],
      );
    }

    final session = ref.read(relaySessionProvider.notifier);

    final [mentionEvents, approvalEvents, agentJobEvents] = await Future.wait([
      // Mentions of me on user-visible channel content.
      session.fetchHistory(
        NostrFilter(
          kinds: const [9, 40002, 1, 45001, 45003],
          tags: {
            '#p': [myPk],
          },
          limit: 50,
        ),
      ),
      // Agent activity and approvals addressed to me.
      session.fetchHistory(
        NostrFilter(
          kinds: const [46010, 46011, 46012],
          tags: {
            '#p': [myPk],
          },
          limit: 20,
        ),
      ),
      // NIP-90 agent job event kinds: request, accepted, progress, result,
      // cancelled, failed.
      session.fetchHistory(
        NostrFilter(
          kinds: const [43001, 43002, 43003, 43004, 43005, 43006],
          tags: {
            '#p': [myPk],
          },
          limit: 20,
        ),
      ),
    ]);

    final mentions = mentionEvents
        .where((e) => e.pubkey.toLowerCase() != myPk.toLowerCase())
        .map((e) => _feedItem(e, category: 'mention'))
        .toList();
    final approvals = approvalEvents
        .map((e) => _feedItem(e, category: 'needs_action'))
        .toList();
    final agentJobs = agentJobEvents
        .map((e) => _feedItem(e, category: 'agent_activity'))
        .toList();

    mentions.sort((a, b) => b.createdAt.compareTo(a.createdAt));
    approvals.sort((a, b) => b.createdAt.compareTo(a.createdAt));
    agentJobs.sort((a, b) => b.createdAt.compareTo(a.createdAt));

    return HomeFeedResponse(
      mentions: mentions,
      needsAction: approvals,
      activity: const [],
      agentActivity: agentJobs,
    );
  }

  FeedItem _feedItem(NostrEvent event, {required String category}) {
    return FeedItem(
      id: event.id,
      kind: event.kind,
      pubkey: event.pubkey,
      content: event.content,
      createdAt: event.createdAt,
      channelId: event.channelId,
      channelName: '',
      tags: event.tags,
      category: category,
    );
  }

  Future<void> refresh() async {
    state = await AsyncValue.guard(_fetch);
  }
}

final activityProvider =
    AsyncNotifierProvider<ActivityNotifier, HomeFeedResponse>(
      ActivityNotifier.new,
    );
