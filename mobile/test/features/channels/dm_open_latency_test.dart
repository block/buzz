import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'opening a DM finishes while the full channel list is stalled',
    () async {
      final keys = nostr.Keys.generate();
      final session = _DmSession(keys.public);
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
      var valid = true;
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

      final opened = await actions
          .openDm(pubkeys: [_peer], waitForChannelList: false)
          .timeout(const Duration(seconds: 2));
      expect(opened.id, _dmId);
      expect(opened.isDm, isTrue);
      expect(opened.isMember, isTrue);
      expect(opened.participantPubkeys, [keys.public, _peer]);
      expect(session.published.single.kind, 41010);
      expect(session.published.single.tags, [
        ['p', _peer],
      ]);

      // The sidebar still reconciles, in the background, once the stalled
      // initial load settles.
      session.stalledList.complete(const []);
      await pumpEventQueue();
      expect(session.listLoads, 2);
      session.stalledList = Completer();

      // A DM the relay confirms but the caller cannot read fails visibly.
      session.member = false;
      await expectLater(
        actions.openDm(pubkeys: [_peer], waitForChannelList: false),
        throwsStateError,
      );

      // Hidden-DM resurfacing keeps waiting for the list that clears its
      // hidden state, so it cannot outrun its own retry coalescing.
      session.member = true;
      var resurfaced = false;
      unawaited(
        actions.openDm(pubkeys: [_peer]).then((_) => resurfaced = true),
      );
      await pumpEventQueue();
      expect(resurfaced, isFalse);

      // A community switch during the targeted read cancels the open.
      session.onRead = () => valid = false;
      await expectLater(
        actions.openDm(pubkeys: [_peer], waitForChannelList: false),
        throwsStateError,
      );
    },
  );
}

const _dmId = 'dm-id';
const _peer =
    '1111111111111111111111111111111111111111111111111111111111111111';

class _DmSession extends RelaySessionNotifier {
  _DmSession(this.self);
  final String self;
  bool member = true;
  void Function()? onRead;
  final published = <NostrEvent>[];
  Completer<List<NostrEvent>> stalledList = Completer();
  final listStarted = Completer<void>();
  int listLoads = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    // Every full channel-list load starts with this membership query.
    expect(filters.single.tags, {
      '#p': [self],
    });
    listLoads++;
    if (!listStarted.isCompleted) listStarted.complete();
    return stalledList.future;
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return NostrEvent(
      id: event.id,
      pubkey: event.pubkey,
      createdAt: event.createdAt,
      kind: event.kind,
      tags: event.tags,
      content: 'response:${jsonEncode({'channel_id': _dmId})}',
      sig: event.sig,
    );
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    expect(filter.kinds, [39000, 39002]);
    expect(filter.tags, {
      '#d': [_dmId],
    });
    onRead?.call();
    return [
      NostrEvent(
        id: 'metadata',
        pubkey: 'relay',
        createdAt: 1,
        kind: 39000,
        tags: [
          ['d', _dmId],
          ['name', 'DM'],
          ['t', 'dm'],
          ['private'],
          ['hidden'],
          ['p', self],
          ['p', _peer],
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
          ['d', _dmId],
          if (member) ['p', self, '', 'member'],
          ['p', _peer, '', 'member'],
        ],
        content: '',
        sig: '',
      ),
    ];
  }
}
