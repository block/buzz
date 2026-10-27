import 'dart:convert';
import 'dart:io';

import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/crypto/nip_oa.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/storage/secure_storage.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/remote_agents_fixture.dart';

final _scope = 'remote-agents:${HostedEnvironment.origin.host}:$fakeSubject';

/// Deterministic test keys.
nostr.Keys _keys(String last) => nostr.Keys('0' * 63 + last);

Community _community({String? pubkey}) => Community.create(
  name: 'Test',
  relayUrl: 'wss://test.${HostedEnvironment.communitySuffix}',
  pubkey: pubkey ?? fakeOwner.public,
  nsec: fakeOwner.nsec,
);

Future<RelayFakeController> _controller(FakeRegistry registry) async {
  await AccountIdentityStore().forSubject(fakeSubject).create(fakeOwner.nsec);
  final controller = RelayFakeController(registry);
  addTearDown(() {
    controller.dispose();
    controller.relayAuth.dispose();
    registry.dispose();
  });
  return controller;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({});
  });

  test('remote agents follow the build-time Builderlab gate', () {
    expect(supportsRemoteAgents(_community()), HostedEnvironment.enabled);
    expect(supportsRemoteAgents(null), isFalse);
    final selfHosted = Community.create(
      name: 'Self hosted',
      relayUrl: 'wss://relay.example.com',
      pubkey: fakeOwner.public,
    );
    expect(supportsRemoteAgents(selfHosted), isFalse);
  });

  test('listing requires a signed-in account bound to the saved key', () async {
    final registry = FakeRegistry(bound: fakeOwner.public, signedIn: false);
    final controller = await _controller(registry);
    registry.listed = [
      {
        'agent_pubkey': fakeAgent,
        'agent_name': 'Scout',
        'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
      },
    ];

    expect(await controller.run(controller.refresh), isTrue);
    expect(registry.calls, isEmpty);
    expect(controller.activeAgents, isEmpty);
    expect(controller.status, contains('Sign in'));

    registry
      ..signedIn = true
      ..bound = nostr.Keys('0' * 63 + '3').public;
    expect(await controller.run(controller.refresh), isFalse);
    expect(registry.calls, isEmpty);
    expect(controller.status, 'Buzz identity: mismatch.');

    registry.bound = fakeOwner.public;
    expect(await controller.run(controller.refresh), isTrue);
    expect(registry.calls, ['list-agents']);
    expect(controller.owner, fakeOwner.public);
    expect(controller.activeAgents, {fakeAgent});

    registry.signedIn = false;
    expect(controller.activeAgents, isEmpty);
  });

  test(
    'failed setup resumes the same identity; instructions stay private',
    () async {
      final registry = FakeRegistry(bound: fakeOwner.public)
        ..failInstructions = true;
      final controller = await _controller(registry);
      final community = _community();

      expect(
        await controller.create(community, 'Scout', instructions: 'x' * 20001),
        isFalse,
      );
      expect(
        await controller.create(_community(pubkey: 'f' * 64), 'Scout'),
        isFalse,
      );
      expect(registry.calls, isEmpty);
      expect(controller.pending, isNull);

      expect(
        await controller.create(
          community,
          'Scout',
          instructions: 'Reply concisely.',
        ),
        isFalse,
      );
      expect(registry.calls, [
        'register-agent',
        'attest-agent',
        'update-agent',
      ]);
      expect(controller.pending?['pubkey'], fakeAgent);
      final prefs = await SharedPreferences.getInstance();
      expect(prefs.getString('$_scope:pending'), isNotNull);
      expect(
        prefs.getString('$_scope:pending'),
        isNot(contains('Reply concisely.')),
      );

      // A fresh controller restores the pending setup from storage.
      final resumed = RelayFakeController(registry);
      addTearDown(() {
        resumed.dispose();
        resumed.relayAuth.dispose();
      });
      registry
        ..failInstructions = false
        ..calls.clear();
      expect(
        await resumed.create(community, 'Other', instructions: 'Other'),
        isTrue,
      );
      expect(registry.calls, [
        'attest-agent',
        'update-agent',
        'add-buzz-subscription',
        'publish-policy',
        'list-agents',
      ]);
      expect(
        registry.bodies['/v3/beekeeper/update-agent']?['agent_instructions'],
        'Reply concisely.',
      );
      expect(
        registry.bodies['/v3/beekeeper/attest-agent']?['community_url'],
        community.relayUrl,
      );
      expect(
        registry.bodies['/v3/beekeeper/add-buzz-subscription']?['community'],
        Uri.parse(community.relayUrl).host,
      );
      expect(resumed.pending, isNull);
      expect(prefs.getString('$_scope:pending'), isNull);
    },
  );

  test(
    'deletion requires confirmed registration absence; disabled stays retryable',
    () async {
      final registry = FakeRegistry(bound: fakeOwner.public);
      final controller = await _controller(registry);
      registry.listed = [
        {'agent_pubkey': fakeAgent, 'agent_id': 'remote-id'},
      ];
      await controller.run(controller.refresh);
      registry.calls.clear();
      var registrationExists = true;
      var current = true;
      Future<bool> remove() => controller.deleteAgent(
        pubkey: fakeAgent,
        community: _community(),
        isCurrent: () => current,
        fetch: (filter) async {
          registry.calls.add('read-registration');
          expect(filter.kinds, [30177]);
          expect(filter.authors, [fakeOwner.public]);
          expect(filter.tags, {
            '#d': [fakeAgent],
          });
          return registrationExists
              ? [
                  NostrEvent(
                    id: 'registration',
                    pubkey: fakeOwner.public,
                    createdAt: 1,
                    kind: 30177,
                    tags: [
                      ['d', fakeAgent],
                    ],
                    content: '{}',
                    sig: '',
                  ),
                ]
              : [];
        },
      );

      controller.rejectRemoval = true;
      expect(await remove(), isFalse);
      expect(registry.calls, ['remove-registration']);
      registry.calls.clear();
      controller.rejectRemoval = false;
      expect(await remove(), isFalse);
      expect(registry.calls, ['remove-registration', 'read-registration']);
      registry
        ..calls.clear()
        ..deleteStatus = 2;
      registrationExists = false;
      expect(await remove(), isFalse);
      expect(controller.status, contains('disabled'));
      expect(controller.agents, hasLength(1));

      // The relay registration is gone, but the listing keeps a retry row.
      registry.listed = [];
      await controller.run(controller.refresh);
      expect(controller.agents.single['agent_id'], 'remote-id');

      registry.calls.clear();
      current = false;
      expect(await remove(), isFalse);
      expect(registry.calls, isEmpty);
      current = true;
      registry.deleteStatus = 'DELETE_AGENT_STATUS_NOT_FOUND';
      expect(await remove(), isTrue);
      expect(registry.calls.last, 'delete-agent');
      expect(controller.agents, isEmpty);
      expect(controller.deletedAgents, {fakeAgent});
      await controller.run(controller.refresh);
      expect(controller.agents, isEmpty);
    },
  );

  test('a setup whose instructions were lost asks for them again', () async {
    final registry = FakeRegistry(bound: fakeOwner.public)
      ..failInstructions = true;
    final controller = await _controller(registry);
    final community = _community();
    expect(
      await controller.create(community, 'Scout', instructions: 'Private.'),
      isFalse,
    );
    final request = controller.pending!['request'];
    // Device-only secure storage is gone; preferences survived (a restore).
    await accountSecureStorage.delete(key: '$_scope:instructions:$request');

    final restarted = RelayFakeController(registry);
    addTearDown(() {
      restarted.dispose();
      restarted.relayAuth.dispose();
    });
    expect(await restarted.run(restarted.refresh), isTrue);
    expect(restarted.pending?['instructionsMissing'], isTrue);
    expect(restarted.status, contains('Enter its instructions again'));

    registry
      ..failInstructions = false
      ..calls.clear();
    expect(
      await restarted.create(community, 'Scout', instructions: 'Again.'),
      isTrue,
    );
    expect(
      registry.bodies['/v3/beekeeper/update-agent']?['agent_instructions'],
      'Again.',
    );
    expect(restarted.pending, isNull);
    final prefs = await SharedPreferences.getInstance();
    expect(prefs.getString('$_scope:pending'), isNull);
  });

  test(
    'registrations use a WebSocket URL for https-saved communities',
    () async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      final kinds = <int>[];
      server.listen((request) async {
        final peer = await WebSocketTransformer.upgrade(request);
        addTearDown(peer.close);
        peer.listen((frame) {
          final data = jsonDecode(frame as String) as List;
          final event = Map<String, dynamic>.from(data[1] as Map);
          if (data.first == 'EVENT') kinds.add(event['kind'] as int);
          peer.add(jsonEncode(['OK', event['id'], true, '']));
        });
        peer.add(jsonEncode(['AUTH', 'challenge']));
      });
      final registry = FakeRegistry(bound: fakeOwner.public);
      await AccountIdentityStore()
          .forSubject(fakeSubject)
          .create(fakeOwner.nsec);
      final controller = RemoteAgentController(
        account: registry,
        keys: AccountIdentityStore(),
        relayAuth: HostedRelayAuth(registry),
      );
      addTearDown(() {
        controller.dispose();
        controller.relayAuth.dispose();
      });
      // Stored the way paired communities may keep it: an HTTP(S) origin.
      final relay = 'http://127.0.0.1:${server.port}';
      await controller.publishPolicy(relay, {
        'pubkey': fakeAgent,
        'name': 'Scout',
      });
      await controller.removePolicy(relay, fakeAgent);
      expect(kinds, [30177, 5]);
    },
  );

  test('a setup without a server identity can be discarded', () async {
    final registry = FakeRegistry(bound: fakeOwner.public)
      ..rejectRegistration = true;
    final controller = await _controller(registry);
    final community = _community();
    final prefs = await SharedPreferences.getInstance();

    expect(
      await controller.create(community, 'Scout', instructions: 'Private.'),
      isFalse,
    );
    expect(controller.status, contains('NAME_TAKEN'));
    expect(controller.canDiscardPending, isTrue);
    // An HTTP rejection is just as definite.
    registry
      ..rejectRegistration = false
      ..refuseRegistration = true;
    expect(await controller.create(community, 'Scout'), isFalse);
    expect(controller.status, contains('rejected'));
    expect(controller.canDiscardPending, isTrue);
    final request = controller.pending!['request'];
    expect(prefs.getString('$_scope:pending'), isNotNull);

    expect(await controller.discardPending(), isTrue);
    expect(controller.pending, isNull);
    expect(prefs.getString('$_scope:pending'), isNull);
    expect(
      await accountSecureStorage.read(key: '$_scope:instructions:$request'),
      isNull,
    );

    // A lost response may hide a committed identity, so it can only resume.
    registry
      ..refuseRegistration = false
      ..loseRegistration = true;
    expect(
      await controller.create(community, 'Scout', instructions: 'Private.'),
      isFalse,
    );
    expect(controller.pending?['pubkey'], isNull);
    expect(controller.canDiscardPending, isFalse);
    expect(await controller.discardPending(), isFalse);
    final lost = controller.pending!['request'];
    expect(jsonDecode(prefs.getString('$_scope:pending')!)['request'], lost);

    // Until the identity is known, any listed agent could be the lost one.
    final other = _keys('3').public;
    registry.listed = [
      ...registry.listed,
      {'agent_pubkey': other, 'agent_id': 'remote-id'},
    ];
    await controller.run(controller.refresh);
    final elsewhere = Community.create(
      name: 'Other',
      relayUrl: 'wss://other.${HostedEnvironment.communitySuffix}',
      pubkey: fakeOwner.public,
      nsec: fakeOwner.nsec,
    );
    Future<bool> remove(String pubkey) => controller.deleteAgent(
      pubkey: pubkey,
      community: elsewhere,
      isCurrent: () => true,
      fetch: (_) async => [],
    );
    expect(await remove(other), isFalse);
    expect(controller.status, contains('Resume the unfinished setup'));

    // Resume from another community reuses the idempotency key and recovers
    // the same identity; finishing it still needs the original community.
    registry
      ..loseRegistration = false
      ..bodies.clear();
    expect(await controller.create(elsewhere, 'Scout'), isFalse);
    expect(controller.status, contains('Agent identity recovered'));
    expect(
      registry.bodies['/v3/beekeeper/register-agent']?['idempotency_key'],
      lost,
    );
    registry.bodies.clear();
    expect(await controller.create(elsewhere, 'Scout'), isFalse);
    expect(controller.status, contains('original community'));
    expect(registry.bodies, isEmpty);
    expect(controller.pending?['pubkey'], fakeAgent);
    expect(await controller.discardPending(), isFalse);
    expect(controller.pending?['pubkey'], fakeAgent);

    // Once known, the unfinished setup blocks deleting only its own agent.
    expect(await remove(fakeAgent), isFalse);
    expect(controller.status, contains('Resume the unfinished setup'));
    expect(await remove(other), isTrue);
    expect(controller.deletedAgents, {other});
    expect(controller.pending?['pubkey'], fakeAgent);
    expect(prefs.getString('$_scope:pending'), isNotNull);
  });

  test('another account never sees the previous registry', () async {
    final registry = FakeRegistry(bound: fakeOwner.public);
    final controller = await _controller(registry);
    registry.listed = [
      {
        'agent_pubkey': fakeAgent,
        'agent_name': 'Scout',
        'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
      },
    ];
    await controller.run(controller.refresh);
    expect(controller.activeAgents, {fakeAgent});

    var notified = 0;
    controller.addListener(() => notified++);
    registry.subject = 'other-owner';
    expect(notified, 1);
    expect(controller.activeAgents, isEmpty);
    // The other account has no saved key, so its registry is never listed.
    expect(await controller.run(controller.refresh), isFalse);
    expect(controller.agents, isEmpty);
    expect(controller.owner, isNull);
  });

  test('a failed registry load retries on the next load', () async {
    final registry = FakeRegistry(bound: 'f' * 64);
    final controller = await _controller(registry);
    final relay = _community().relayUrl;
    await controller.load(relay);
    expect(controller.status, 'Buzz identity: mismatch.');
    registry.bound = fakeOwner.public;
    await controller.load(relay);
    expect(registry.calls, ['list-agents']);
    await controller.load(relay);
    expect(registry.calls, ['list-agents']);
  });

  test(
    'availability requires an owned profile and a live registration',
    () async {
      String key(String last) => _keys(last).public;
      final other = key('3');
      final elsewhere = key('5');
      final forged = signedAgentProfile(_keys('8'), fakeOwner).toJson()
        ..['sig'] = '0' * 128;
      final profiles = [
        signedAgentProfile(fakeAgentKeys, fakeOwner),
        // Attested by someone else, so not this owner's agent.
        signedAgentProfile(_keys('3'), _keys('4')),
        // Signed conditions that exclude this profile, and a forged envelope.
        signedAgentProfile(_keys('6'), fakeOwner, conditions: 'kind=1'),
        signedAgentProfile(_keys('7'), fakeOwner, conditions: 'created_at<50'),
        NostrEvent.fromJson(forged),
      ];
      var registrations = <NostrEvent>[];
      Future<List<NostrEvent>> fetch(NostrFilter filter) async {
        if (filter.kinds.single == 0) return profiles;
        expect(filter.kinds, [30177]);
        expect(filter.authors, [fakeOwner.public]);
        expect(filter.tags, {
          '#d': [fakeAgent],
        });
        return registrations;
      }

      final keys = {fakeAgent, other, elsewhere, key('6'), key('7'), key('8')};
      // The list keeps owned agents, so an interrupted delete stays retryable.
      expect(await ownedCommunityAgents(keys, fakeOwner.public, fetch), {
        fakeAgent,
      });
      expect(await liveCommunityAgents(keys, fakeOwner.public, fetch), isEmpty);
      registrations = [_registration(fakeAgent)];
      expect(await liveCommunityAgents(keys, fakeOwner.public, fetch), {
        fakeAgent,
      });
      expect(await liveCommunityAgents(keys, null, fetch), isEmpty);
    },
  );

  test('presence stays local until the agent registration is live', () async {
    final registry = FakeRegistry(bound: fakeOwner.public)
      ..listed = [
        {
          'agent_pubkey': fakeAgent,
          'agent_id': 'remote-id',
          'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
        },
      ];
    final controller = await _controller(registry);
    final relay = _AgentRelay();
    final container = ProviderContainer(
      overrides: [
        presenceCacheProvider.overrideWith(_AgentPresence.new),
        activeCommunityProvider.overrideWith((ref) async => _community()),
        remoteAgentControllerProvider.overrideWithValue(controller),
        relaySessionProvider.overrideWith(() => relay),
      ],
    );
    addTearDown(container.dispose);
    await container.read(activeCommunityProvider.future);
    await controller.run(controller.refresh);

    expect(await container.read(availableRemoteAgentsProvider.future), isEmpty);
    expect(container.read(presentedPresenceProvider)[fakeAgent], 'offline');

    relay.live = true;
    container.invalidate(availableRemoteAgentsProvider);
    expect(await container.read(availableRemoteAgentsProvider.future), {
      fakeAgent,
    });
    expect(container.read(presentedPresenceProvider)[fakeAgent], 'online');

    // Removing the registration hides it even when the runtime stays ACTIVE.
    registry.deleteStatus = 2;
    relay.live = false;
    expect(
      await controller.deleteAgent(
        pubkey: fakeAgent,
        community: _community(),
        isCurrent: () => true,
        fetch: (_) async => [],
      ),
      isFalse,
    );
    expect(controller.activeAgents, {fakeAgent});
    expect(await container.read(availableRemoteAgentsProvider.future), isEmpty);
    expect(container.read(presentedPresenceProvider)[fakeAgent], 'offline');

    // Publishing a registration for the still-ACTIVE agent shows it again.
    relay.live = true;
    expect(await controller.create(_community(), 'Scout'), isTrue);
    expect(controller.activeAgents, {fakeAgent});
    expect(await container.read(availableRemoteAgentsProvider.future), {
      fakeAgent,
    });
    expect(container.read(presentedPresenceProvider)[fakeAgent], 'online');
  }, skip: !HostedEnvironment.enabled);

  test('owner proof verifies for only the provisioned agent key', () {
    final other = nostr.Keys('0' * 63 + '3');
    final auth = signOwnerAuthorization(fakeOwner, fakeAgent);
    expect(verifiedOaOwnerPubkey([auth], fakeAgent), fakeOwner.public);
    expect(verifiedOaOwnerPubkey([auth], other.public), isNull);
    expect(
      () => signOwnerAuthorization(fakeOwner, fakeOwner.public),
      throwsStateError,
    );
  });

  test('disabled and missing statuses cannot count as activation', () {
    requireStatus({'status': 'ATTEST_AGENT_STATUS_ACTIVE'}, 'ACTIVE', 1);
    requireStatus({'status': 1}, 'ACTIVE', 1);
    for (final result in [
      {'status': 'ATTEST_AGENT_STATUS_FEATURE_DISABLED'},
      <String, dynamic>{},
    ]) {
      expect(() => requireStatus(result, 'ACTIVE', 1), throwsStateError);
    }
  });

  test(
    'remote availability shows Online without changing local presence',
    () async {
      final container = ProviderContainer(
        overrides: [
          presenceCacheProvider.overrideWith(_Presence.new),
          availableRemoteAgentsProvider.overrideWith((ref) async => {'remote'}),
        ],
      );
      addTearDown(container.dispose);
      await container.read(availableRemoteAgentsProvider.future);
      expect(container.read(presentedPresenceProvider), {
        'remote': 'online',
        'local': 'away',
      });
      expect(container.read(presenceCacheProvider)['remote'], 'offline');
    },
  );
}

class _Presence extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {'remote': 'offline', 'local': 'away'};
}

class _AgentPresence extends PresenceCacheNotifier {
  @override
  Map<String, String> build() => {fakeAgent: 'offline'};
}

NostrEvent _registration(String agent) => NostrEvent(
  id: 'registration-$agent',
  pubkey: fakeOwner.public,
  createdAt: 1,
  kind: 30177,
  tags: [
    ['d', agent],
  ],
  content: '{}',
  sig: '',
);

/// Serves the owner's agent profile, and its registration once [live].
class _AgentRelay extends RelaySessionNotifier {
  bool live = false;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => switch (filter.kinds.single) {
    0 => [signedAgentProfile(fakeAgentKeys, fakeOwner)],
    30177 when live => [_registration(fakeAgent)],
    _ => [],
  };
}
