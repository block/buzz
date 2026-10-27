import 'dart:async';

import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'Solo creation finishes while the full channel list is stalled',
    () async {
      final keys = nostr.Keys.generate();
      final session = _SoloSession(keys.public);
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          myPubkeyProvider.overrideWithValue(keys.public),
        ],
      );
      addTearDown(container.dispose);
      container.listen(channelsProvider, (_, _) {});
      await session.listStarted.future;
      final actions = container.read(
        Provider(
          (ref) => ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: keys.public,
          ),
        ),
      );

      expect(await actions.findJoinedChannel('solo-id'), isNull);
      final created = await actions
          .createChannel(
            channelId: 'solo-id',
            name: 'New chat',
            channelType: 'stream',
            visibility: 'private',
            labels: const ['buzz.solo'],
            waitForChannelList: false,
          )
          .timeout(const Duration(seconds: 2));
      expect(session.stalledList.isCompleted, isFalse);
      expect(container.read(channelsProvider).isLoading, isTrue);
      expect(created.isPersonalChat(keys.public, const []), isTrue);
      expect(created.isMember, isTrue);
      expect(created.memberCount, 1);
      expect(session.reads, 2);
      expect(session.published.single.kind, 9007);
      expect(
        session.published.single.tags,
        contains(equals(['t', 'buzz.solo'])),
      );
      expect(session.published.single.tags.any((t) => t.first == 'P'), isFalse);
      // A retry reads the same channel, without a broad membership query.
      expect((await actions.findJoinedChannel('solo-id'))?.id, created.id);
      expect(session.published, hasLength(1));
    },
  );

  test(
    'targeted recovery requires membership and preserves failures',
    () async {
      final keys = nostr.Keys.generate();
      final session = _SoloSession('another-owner')..exists = true;
      var valid = true;
      final container = ProviderContainer();
      addTearDown(container.dispose);
      final actions = container.read(
        Provider(
          (ref) => ChannelActions(
            ref: ref,
            session: session,
            signedEventRelay: SignedEventRelay(
              session: session,
              nsec: keys.nsec,
            ),
            currentPubkey: keys.public,
            isCommunityValid: () => valid,
          ),
        ),
      );
      expect(await actions.findJoinedChannel('solo-id'), isNull);
      session.failure = TimeoutException('relay unavailable');
      await expectLater(
        actions.findJoinedChannel('solo-id'),
        throwsA(isA<TimeoutException>()),
      );
      session.failure = null;
      session.onRead = () => valid = false;
      await expectLater(actions.findJoinedChannel('solo-id'), throwsStateError);
      expect(session.published, isEmpty);
    },
  );
}

class _SoloSession extends RelaySessionNotifier {
  _SoloSession(this.owner);
  final String owner;
  bool exists = false;
  int reads = 0;
  Object? failure;
  void Function()? onRead;
  final published = <NostrEvent>[];
  final stalledList = Completer<List<NostrEvent>>();
  final listStarted = Completer<void>();

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    expect(filters.single.tags, {
      '#p': [owner],
    });
    if (!listStarted.isCompleted) listStarted.complete();
    return stalledList.future;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    exists = true;
    return event;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    reads++;
    expect(filter.kinds, [39000, 39002]);
    expect(filter.tags, {
      '#d': ['solo-id'],
    });
    expect(filter.limit, 2);
    onRead?.call();
    if (failure != null) throw failure!;
    if (!exists) return [];
    return [
      NostrEvent(
        id: 'metadata',
        pubkey: 'relay',
        createdAt: 1,
        kind: 39000,
        tags: [
          ['d', 'solo-id'],
          ['name', 'New chat'],
          ['t', 'stream'],
          ['t', 'buzz.solo'],
          ['private'],
          ['P', owner],
        ],
        content: '',
        sig: '',
      ),
      NostrEvent(
        id: 'roster',
        pubkey: 'relay',
        createdAt: 1,
        kind: 39002,
        tags: [
          ['d', 'solo-id'],
          ['p', owner, '', 'owner'],
        ],
        content: '',
        sig: '',
      ),
    ];
  }
}
