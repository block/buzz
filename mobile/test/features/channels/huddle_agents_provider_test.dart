import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/huddle_agents_provider.dart';
import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('role bot and trusted managed-agent records both make agents', () async {
    final relay = _HuddleRelaySession(
      members: {
        _owner.public: 'owner',
        _viewer.public: 'member',
        _botAgent: 'bot',
        _ownerAgent: 'member',
        _viewerAgent: 'member',
        _strangerAgent: 'member',
        _person: 'member',
      },
      records: [
        _record(_owner, _ownerAgent),
        _record(_viewer, _viewerAgent),
        // A member who owns nothing in this Huddle cannot mark an agent.
        _record(_stranger, _strangerAgent),
      ],
    );
    final container = _container(relay);

    expect(await _settledAgents(container), {
      _botAgent,
      _ownerAgent,
      _viewerAgent,
    });
    final query = relay.recordFilters.single;
    expect(query.authors, unorderedEquals([_viewer.public, _owner.public]));
    expect(query.tags['#d'], isNot(contains(_botAgent)));
    expect(query.tags['#d'], contains(_ownerAgent));
  });

  test('a failed record read keeps role bot agents only', () async {
    final relay = _HuddleRelaySession(
      members: {
        _owner.public: 'owner',
        _botAgent: 'bot',
        _ownerAgent: 'member',
      },
      recordError: Exception('relay timeout'),
    );
    final container = _container(relay);

    expect(await _settledAgents(container), {_botAgent});
    expect(relay.recordFilters, hasLength(1));
  });

  test('a reload keeps record agents only while they are members', () async {
    final relay = _HuddleRelaySession(
      members: {
        _owner.public: 'owner',
        _botAgent: 'bot',
        _ownerAgent: 'member',
        _viewerAgent: 'member',
      },
      records: [_record(_owner, _ownerAgent), _record(_owner, _viewerAgent)],
    );
    final container = _container(relay);
    expect(await _settledAgents(container), {
      _botAgent,
      _ownerAgent,
      _viewerAgent,
    });

    // _ownerAgent leaves; the record reload is slow and then fails.
    relay.members = {
      _owner.public: 'owner',
      _botAgent: 'bot',
      _viewerAgent: 'member',
    };
    relay.recordGate = Completer<void>();
    relay.recordError = Exception('relay timeout');
    container.invalidate(channelMembersProvider(_channelId));
    expect(await _agentsNow(container), {_botAgent, _viewerAgent});
    expect(relay.recordFilters, hasLength(2));

    relay.recordGate!.complete();
    expect(await _settledAgents(container), {_botAgent, _viewerAgent});

    // The next successful read is authoritative again.
    relay.recordError = null;
    relay.recordGate = null;
    relay.members = {_owner.public: 'owner', _botAgent: 'bot'};
    container.invalidate(channelMembersProvider(_channelId));
    expect(await _settledAgents(container), {_botAgent});
  });

  test(
    'a relay disconnect keeps record agents until a connected read',
    () async {
      final relay = _HuddleRelaySession(
        members: {
          _owner.public: 'owner',
          _botAgent: 'bot',
          _ownerAgent: 'member',
          _viewerAgent: 'member',
        },
        records: [_record(_owner, _ownerAgent), _record(_owner, _viewerAgent)],
      );
      final container = _container(relay);
      expect(await _settledAgents(container), {
        _botAgent,
        _ownerAgent,
        _viewerAgent,
      });

      // While disconnected, the last records still apply.
      relay.setStatus(SessionStatus.reconnecting);
      expect(await _settledAgents(container), {
        _botAgent,
        _ownerAgent,
        _viewerAgent,
      });
      expect(relay.recordFilters, hasLength(1));

      // After reconnecting, a successful read is authoritative again.
      relay.records = [_record(_owner, _ownerAgent)];
      relay.setStatus(SessionStatus.connected);
      expect(await _settledAgents(container), {_botAgent, _ownerAgent});
      expect(relay.recordFilters, hasLength(2));
    },
  );

  test('records read for another account do not apply', () async {
    final relay = _HuddleRelaySession(
      members: {_owner.public: 'owner', _viewerAgent: 'member'},
      // Only the viewer vouches for this agent.
      records: [_record(_viewer, _viewerAgent)],
    );
    final container = _container(relay);
    final viewer = container.read(_viewerOverride.notifier);
    // Load the Huddle once as the other account, so its member snapshot is
    // cached, then as the viewer.
    viewer.set(_stranger.public);
    expect(await _settledAgents(container), isEmpty);
    viewer.set(_viewer.public);
    expect(await _settledAgents(container), {_viewerAgent});

    // Switching account while disconnected must not carry the viewer's
    // records over to the other account's cached members.
    relay.setStatus(SessionStatus.reconnecting);
    viewer.set(_stranger.public);
    expect(await _settledAgents(container), isEmpty);
  });

  test('last-human count leaves out agents known by their record', () async {
    final relay = _HuddleRelaySession(
      members: {
        _owner.public: 'owner',
        _viewer.public: 'member',
        _botAgent: 'bot',
        _ownerAgent: 'member',
        _strangerAgent: 'member',
      },
      records: [
        _record(_owner, _ownerAgent),
        _record(_stranger, _strangerAgent),
      ],
    );
    final container = _container(relay);

    // The owner, the viewer and the agent that only a stranger vouched for.
    expect(await container.read(huddleHumanCountProvider)(_channelId), 3);
  });

  test('last-human count fails when the records cannot be read', () async {
    final relay = _HuddleRelaySession(
      members: {_owner.public: 'owner', _ownerAgent: 'member'},
      recordError: Exception('relay timeout'),
    );
    final container = _container(relay);

    // Leave treats a failed count as "another person remains".
    await expectLater(
      container.read(huddleHumanCountProvider)(_channelId),
      throwsException,
    );
  });

  test('records need a valid signature, the d tag and kind', () {
    final valid = _record(_owner, _ownerAgent);
    final forged = NostrEvent(
      id: valid.id,
      pubkey: valid.pubkey,
      createdAt: valid.createdAt,
      kind: valid.kind,
      tags: [
        ['d', _viewerAgent],
      ],
      content: valid.content,
      sig: valid.sig,
    );

    expect(
      managedAgentPubkeysFromRecords(
        events: [valid, forged, _record(_owner, _person, kind: 30078)],
        trustedAuthors: [_owner.public],
        candidates: [_ownerAgent, _viewerAgent, _person],
      ),
      {_ownerAgent},
    );
  });

  test('trusted authors are the viewer and the channel owners', () {
    final members = [
      _member(_owner.public, 'owner'),
      _member(_stranger.public, 'admin'),
      _member(_botAgent, 'bot'),
    ];

    expect(
      trustedManagedAgentRecordAuthors(
        members: members,
        viewerPubkey: _viewer.public.toUpperCase(),
      ),
      [_viewer.public, _owner.public],
    );
    expect(managedAgentRecordCandidates(members), [
      _owner.public,
      _stranger.public,
    ]);
  });
}

final _viewerOverride = NotifierProvider<_Viewer, String?>(_Viewer.new);

class _Viewer extends Notifier<String?> {
  @override
  String? build() => _viewer.public;

  void set(String? pubkey) => state = pubkey;
}

ProviderContainer _container(_HuddleRelaySession relay) {
  final container = ProviderContainer(
    retry: (_, _) => null,
    overrides: [
      relaySessionProvider.overrideWith(() => relay),
      myPubkeyProvider.overrideWith((ref) => ref.watch(_viewerOverride)),
    ],
  );
  addTearDown(container.dispose);
  final keepAlive = container.listen(
    huddleAgentPubkeysProvider(_channelId),
    (_, _) {},
    fireImmediately: true,
  );
  addTearDown(keepAlive.close);
  return container;
}

/// The agents once the record read has finished, including its isolate.
Future<Set<String>> _settledAgents(ProviderContainer container) async {
  await _pump();
  try {
    await container.read(
      huddleManagedAgentRecordPubkeysProvider(_channelId).future,
    );
  } catch (_) {}
  return _agentsNow(container);
}

/// The agents while a record read may still be in flight.
Future<Set<String>> _agentsNow(ProviderContainer container) async {
  await _pump();
  return container.read(huddleAgentPubkeysProvider(_channelId));
}

Future<void> _pump() async {
  for (var i = 0; i < 10; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

MemberEntry _member(String pubkey, String role) =>
    MemberEntry(pubkey: pubkey, role: role);

class _HuddleRelaySession extends RelaySessionNotifier {
  Map<String, String> members;
  List<NostrEvent> records;
  Object? recordError;
  Completer<void>? recordGate;
  final List<NostrFilter> recordFilters = [];

  _HuddleRelaySession({
    required this.members,
    this.records = const [],
    this.recordError,
  });

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(39002)) {
      return [
        NostrEvent(
          id: 'membership',
          pubkey: 'relay',
          createdAt: 1,
          kind: 39002,
          tags: [
            ['d', _channelId],
            for (final entry in members.entries)
              ['p', entry.key, 'wss://relay.example', entry.value],
          ],
          content: '',
          sig: 'sig',
        ),
      ];
    }
    if (filter.kinds.contains(managedAgentRecordKind)) {
      recordFilters.add(filter);
      if (recordGate case final gate?) await gate.future;
      if (recordError case final error?) throw error;
      return records;
    }
    return const [];
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async {
    onStatusChanged(RelaySubscriptionStatus.ready);
    return () {};
  }
}

NostrEvent _record(nostr.Keys author, String agent, {int kind = 30177}) {
  final event = nostr.Event.from(
    kind: kind,
    content: '{}',
    tags: [
      ['d', agent],
    ],
    secretKey: author.secret,
    createdAt: 1792680000,
  );
  return NostrEvent(
    id: event.id,
    pubkey: event.pubkey,
    createdAt: event.createdAt,
    kind: event.kind,
    tags: event.tags,
    content: event.content,
    sig: event.sig,
  );
}

const _channelId = '22222222-2222-4222-8222-222222222222';
final _owner = nostr.Keys('1' * 64);
final _viewer = nostr.Keys('2' * 64);
final _stranger = nostr.Keys('3' * 64);
final _botAgent = 'b' * 64;
final _ownerAgent = 'c' * 64;
final _viewerAgent = 'd' * 64;
final _strangerAgent = 'e' * 64;
final _person = 'f' * 64;
