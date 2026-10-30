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
      // Composers find it before the list has it, so sends there can check
      // mentions and address the DM.
      expect(
        container.read(channelsProvider.notifier).channelById(_dmId)?.id,
        _dmId,
      );

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

      // A reopen whose unhide snapshot the relay failed to publish fails
      // visibly instead of opening a DM the sidebar still hides.
      session.member = true;
      session.hidden = {_dmId};
      await expectLater(
        actions.openDm(pubkeys: [_peer], waitForChannelList: false),
        throwsStateError,
      );
      expect(session.visibilityReads, greaterThan(0));
      // A newly created DM cannot be in an older snapshot, so it skips that
      // read and cannot fail on it after the relay created the DM.
      session.created = true;
      final reads = session.visibilityReads;
      expect(
        (await actions.openDm(pubkeys: [_peer], waitForChannelList: false)).id,
        _dmId,
      );
      expect(session.visibilityReads, reads);
      session.created = false;
      session.hidden = {};

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
  Set<String> hidden = {};
  bool created = false;
  int visibilityReads = 0;
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
      content:
          'response:${jsonEncode({'channel_id': _dmId, 'created': created})}',
      sig: event.sig,
    );
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.single == EventKind.dmVisibility) {
      visibilityReads++;
      expect(filter.tags, {
        '#p': [self],
      });
      return [
        NostrEvent(
          id: 'visibility',
          pubkey: 'relay',
          createdAt: 1,
          kind: EventKind.dmVisibility,
          tags: [
            ['p', self],
            for (final id in hidden) ['h', id],
          ],
          content: '',
          sig: '',
        ),
      ];
    }
    expect(filter.tags, {
      '#d': [_dmId],
    });
    onRead?.call();
    // Serve like a relay: newest first, one limit across every kind. A stale
    // metadata revision from before the d_tag backfill still lingers.
    final events = [
      _event(39000, 3, [
        ['name', 'DM'],
        ['t', 'dm'],
        ['private'],
        ['hidden'],
        ['p', self],
        ['p', _peer],
      ]),
      _event(39000, 2, [
        ['name', 'DM'],
        ['t', 'dm'],
        ['private'],
      ]),
      _event(39002, 1, [
        if (member) ['p', self, '', 'member'],
        ['p', _peer, '', 'member'],
      ]),
    ].where((e) => filter.kinds.contains(e.kind)).toList();
    return events.take(filter.limit).toList();
  }
}

NostrEvent _event(int kind, int createdAt, List<List<String>> tags) =>
    NostrEvent(
      id: '$kind-$createdAt',
      pubkey: 'relay',
      createdAt: createdAt,
      kind: kind,
      tags: [
        ['d', _dmId],
        ...tags,
      ],
      content: '',
      sig: '',
    );
