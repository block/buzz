import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

import 'package:buzz/app.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';

import '../../shared/community/community_storage_test.dart';

void main() {
  test(
    'legacy invite entry refuses staging before key generation or claim',
    () async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      var generated = 0;
      var requests = 0;
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          inviteKeyGeneratorProvider.overrideWithValue(() {
            generated++;
            return nostr.Keys.generate();
          }),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((_) async {
              requests++;
              return http.Response('{}', 500);
            }),
          ),
        ],
      );
      addTearDown(container.dispose);
      await container.read(communityListProvider.future);
      final notifier = container.read(inviteJoinProvider.notifier);
      await notifier.prepare(
        const InviteDeepLink(
          relayUrl: 'wss://team.communities.public.builderlabstaging.xyz',
          code: 'fixture',
        ),
      );
      await notifier.confirmJoin();
      expect(container.read(inviteJoinProvider).status, InviteJoinStatus.error);
      expect(generated, 0);
      expect(requests, 0);
      expect(await storage.loadAll(), isEmpty);
    },
    skip: !HostedEnvironment.requiresAuth(
      'team.communities.public.builderlabstaging.xyz',
    ),
  );

  for (final existingRelayUrl in [
    'wss://relay.example.com',
    'https://relay.example.com',
  ]) {
    test(
      'same-relay invite switches existing $existingRelayUrl before keygen or claim',
      () async {
        var generatedKeys = 0;
        var claimRequests = 0;
        final storage = CommunityStorage(secure: FakeSecureStorage());
        final existing = Community(
          id: 'existing-id',
          name: 'Existing',
          relayUrl: existingRelayUrl,
          pubkey: 'old-pubkey',
          nsec: 'old-nsec',
          addedAt: DateTime.utc(2026),
        );
        await storage.save(existing);
        final auth = _RecordingAuthNotifier();
        final container = ProviderContainer(
          overrides: [
            communityStorageProvider.overrideWithValue(storage),
            authProvider.overrideWith(() => auth),
            inviteKeyGeneratorProvider.overrideWithValue(() {
              generatedKeys++;
              return nostr.Keys.generate();
            }),
            inviteJoinHttpClientProvider.overrideWithValue(
              http_testing.MockClient((request) async {
                claimRequests++;
                return http.Response('{}', 500);
              }),
            ),
          ],
        );
        addTearDown(container.dispose);
        await container.read(communityListProvider.future);

        await container
            .read(inviteJoinProvider.notifier)
            .prepare(
              const InviteDeepLink(
                relayUrl: 'wss://relay.example.com',
                code: 'code',
              ),
            );

        final state = container.read(inviteJoinProvider);
        final stored = (await storage.loadAll()).single;
        expect(state.status, InviteJoinStatus.switchedExisting);
        expect(await storage.loadActiveId(), existing.id);
        expect(stored.relayUrl, existingRelayUrl);
        expect(stored.pubkey, 'old-pubkey');
        expect(stored.nsec, 'old-nsec');
        expect(generatedKeys, 0);
        expect(claimRequests, 0);
        expect(auth.authenticatedCommunities, isEmpty);
      },
    );
  }

  test(
    'claim posts with freshly-generated key and stores joined community',
    () async {
      final keys = nostr.Keys.generate();
      http.Request? capturedRequest;
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final auth = _RecordingAuthNotifier();
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          authProvider.overrideWith(() => auth),
          inviteKeyGeneratorProvider.overrideWithValue(() => keys),
          inviteJoinRecoveryProvider.overrideWithValue(
            (_) => _successfulRecovery(),
          ),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              capturedRequest = request;
              return http.Response(
                jsonEncode({
                  'status': 'joined',
                  'community_id': 'community-id',
                  'host': 'relay.example.com',
                  'role': 'member',
                }),
                200,
              );
            }),
          ),
        ],
      );
      addTearDown(container.dispose);

      await container
          .read(inviteJoinProvider.notifier)
          .prepare(
            const InviteDeepLink(
              relayUrl: 'wss://relay.example.com',
              code: 'code',
            ),
          );
      expect(
        container.read(inviteJoinProvider).status,
        InviteJoinStatus.confirming,
      );

      await container.read(inviteJoinProvider.notifier).confirmJoin();

      final state = container.read(inviteJoinProvider);
      expect(state.status, InviteJoinStatus.success);
      expect(state.focusChannelId, 'welcome-everyone-id');
      expect(capturedRequest, isNotNull);
      expect(
        capturedRequest!.url.toString(),
        'https://relay.example.com/api/invites/claim',
      );
      expect(capturedRequest!.body, jsonEncode({'code': 'code'}));
      final authHeader = capturedRequest!.headers['Authorization'];
      expect(authHeader, startsWith('Nostr '));
      expect(capturedRequest!.followRedirects, isFalse);
      final encoded = authHeader!.substring('Nostr '.length);
      final authEvent =
          jsonDecode(
                utf8.decode(base64Url.decode(base64Url.normalize(encoded))),
              )
              as Map<String, dynamic>;
      final tags = (authEvent['tags'] as List<dynamic>)
          .map((tag) => (tag as List<dynamic>).cast<String>())
          .toList();
      final payloadHash = SHA256Digest()
          .process(capturedRequest!.bodyBytes)
          .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
          .join();
      expect(authEvent['kind'], 27235);
      expect(tags, contains(equals(['u', capturedRequest!.url.toString()])));
      expect(tags, contains(equals(['method', 'POST'])));
      expect(tags, contains(equals(['payload', payloadHash])));
      expect(auth.authenticatedCommunities, hasLength(1));
      expect(
        auth.authenticatedCommunities.single.relayUrl,
        'wss://relay.example.com',
      );
      expect(auth.authenticatedCommunities.single.pubkey, keys.public);
      expect(auth.authenticatedCommunities.single.nsec, keys.nsec);
      expect(
        auth.authenticatedCommunities.single.sensitiveActionPolicy,
        SensitiveActionPolicy.disabledByUser,
      );
    },
  );

  test('prepare rejects an unsafe relay before showing confirmation', () async {
    final container = ProviderContainer();
    addTearDown(container.dispose);

    await expectLater(
      container
          .read(inviteJoinProvider.notifier)
          .prepare(
            const InviteDeepLink(relayUrl: 'wss://127.0.0.1', code: 'code'),
          ),
      throwsFormatException,
    );
    expect(container.read(inviteJoinProvider).status, InviteJoinStatus.idle);
  });

  test('starter channel ids match desktop for the same relay scope', () async {
    expect(
      desktopStarterChannelId(
        relayHttpOrigin: 'https://relay.example.com/',
        slug: 'general',
      ),
      '9ed9563a-84d6-586d-8007-ae294a6dfdaf',
    );
    expect(
      desktopStarterChannelId(
        relayHttpOrigin: 'https://relay.example.com',
        slug: 'welcome-everyone',
      ),
      '1e288e10-2f7d-5c2c-9a9f-dee58c8daa7a',
    );
  });

  test(
    'invite recovery joins existing public general and welcome-everyone',
    () async {
      final joined = <String>[];
      final recovery = MobileInviteJoinRecovery(
        loadChannels: () async => [
          _channel(id: 'general-id', name: ' General '),
          _channel(id: 'welcome-id', name: 'WELCOME-EVERYONE'),
          _channel(
            id: 'private-welcome-id',
            name: 'Welcome',
            visibility: 'private',
          ),
        ],
        fetchChannel: (_) async => fail('no direct lookup expected'),
        createChannel:
            ({
              required channelId,
              required name,
              required channelType,
              required visibility,
              description,
              ttlSeconds,
            }) async => throw StateError('starters already exist'),
        joinChannel: (channelId) async => joined.add(channelId),
        relayHttpOrigin: 'https://relay.example.com',
      );

      final focusChannelId = await recovery.ensureStarterChannels();

      expect(joined, ['general-id', 'welcome-id']);
      expect(focusChannelId, 'welcome-id');
    },
  );

  test(
    'invite recovery creates missing public starters with desktop ids',
    () async {
      final created = <String, Map<String, Object?>>{};
      final recovery = MobileInviteJoinRecovery(
        loadChannels: () async => const [],
        fetchChannel: (_) async => fail('no direct lookup expected'),
        createChannel:
            ({
              required channelId,
              required name,
              required channelType,
              required visibility,
              description,
              ttlSeconds,
            }) async {
              created[name] = {
                'id': channelId,
                'channelType': channelType,
                'visibility': visibility,
                'description': description,
                'ttlSeconds': ttlSeconds,
              };
              return _channel(id: channelId, name: name, isMember: true);
            },
        joinChannel: (_) async => fail('creator is already a member'),
        relayHttpOrigin: 'https://relay.example.com/',
      );

      final focusChannelId = await recovery.ensureStarterChannels();

      expect(created.keys, ['general', 'welcome-everyone']);
      expect(created['general'], {
        'id': '9ed9563a-84d6-586d-8007-ae294a6dfdaf',
        'channelType': 'stream',
        'visibility': 'open',
        'description': 'General conversation and community updates.',
        'ttlSeconds': null,
      });
      expect(created['welcome-everyone'], {
        'id': '1e288e10-2f7d-5c2c-9a9f-dee58c8daa7a',
        'channelType': 'stream',
        'visibility': 'open',
        'description':
            'Say hi, ask a question, or share what brought you here.',
        'ttlSeconds': null,
      });
      expect(focusChannelId, '1e288e10-2f7d-5c2c-9a9f-dee58c8daa7a');
    },
  );

  test(
    'duplicate starter creation converges and joins relay channels',
    () async {
      var loadCount = 0;
      final joined = <String>[];
      final recovery = MobileInviteJoinRecovery(
        loadChannels: () async {
          loadCount++;
          if (loadCount == 1) return const [];
          return [
            _channel(id: 'relay-general', name: 'general'),
            _channel(id: 'relay-welcome', name: 'welcome-everyone'),
          ];
        },
        fetchChannel: (_) async => fail('no direct lookup expected'),
        createChannel:
            ({
              required channelId,
              required name,
              required channelType,
              required visibility,
              description,
              ttlSeconds,
            }) async => throw Exception(
              'relay rejected event: duplicate: channel already exists',
            ),
        joinChannel: (channelId) async => joined.add(channelId),
        relayHttpOrigin: 'https://relay.example.com',
      );

      final focusChannelId = await recovery.ensureStarterChannels();

      expect(loadCount, 2);
      expect(joined, ['relay-general', 'relay-welcome']);
      expect(focusChannelId, 'relay-welcome');
    },
  );

  test(
    'taken starter names are skipped or converged instead of failing the invite',
    () async {
      const origin = 'https://relay.example.com';
      const refusal = 'invalid: an open channel named "general" already exists';
      const duplicateId = 'duplicate: channel already exists';
      final generalId = desktopStarterChannelId(
        relayHttpOrigin: origin,
        slug: 'general',
      );
      final welcomeId = desktopStarterChannelId(
        relayHttpOrigin: origin,
        slug: 'welcome-everyone',
      );
      ({
        List<String> created,
        List<String> joined,
        List<String> lookups,
        MobileInviteJoinRecovery recovery,
        int Function() loads,
      })
      starterSetup({
        required List<Channel> Function(int load) channelsOnLoad,
        required String refusalError,
        Channel? atId,
        Set<String> refused = const {'general'},
        bool Function()? isScopeCurrent,
        void Function()? onLookup,
      }) {
        final created = <String>[];
        final joined = <String>[];
        final lookups = <String>[];
        var loadCount = 0;
        final recovery = MobileInviteJoinRecovery(
          loadChannels: () async => channelsOnLoad(++loadCount),
          fetchChannel: (channelId) async {
            lookups.add(channelId);
            onLookup?.call();
            return atId;
          },
          createChannel:
              ({
                required channelId,
                required name,
                required channelType,
                required visibility,
                description,
                ttlSeconds,
              }) async {
                created.add(name);
                if (refused.contains(name)) {
                  throw Exception(refusalError);
                }
                return _channel(id: channelId, name: name, isMember: true);
              },
          joinChannel: (channelId) async => joined.add(channelId),
          relayHttpOrigin: origin,
          isScopeCurrent: isScopeCurrent,
        );
        return (
          created: created,
          joined: joined,
          lookups: lookups,
          recovery: recovery,
          loads: () => loadCount,
        );
      }

      // A holder the directory shows cannot serve as the starter is neither
      // joined nor retried; the other starter is still created and focused.
      final unusableHolders = [
        (
          _channel(
            id: 'old-general',
            name: 'General',
            archivedAt: DateTime.utc(2025),
          ),
          refusal,
        ),
        (
          _channel(id: 'forum-general', name: 'general', channelType: 'forum'),
          refusal,
        ),
        (
          _channel(
            id: generalId,
            name: 'general',
            archivedAt: DateTime.utc(2025),
          ),
          duplicateId,
        ),
      ];
      for (final (holder, error) in unusableHolders) {
        final setup = starterSetup(
          channelsOnLoad: (_) => [holder],
          refusalError: error,
        );

        final focusChannelId = await setup.recovery.ensureStarterChannels();

        expect(setup.created, [
          'general',
          'welcome-everyone',
        ], reason: holder.id);
        expect(setup.joined, isEmpty, reason: holder.id);
        expect(setup.loads(), 2, reason: holder.id);
        expect(focusChannelId, welcomeId, reason: holder.id);
      }

      // An archived channel at the starter's own id, which the directory does
      // not return, is found by a direct lookup and skipped, not rethrown.
      final archivedAtId = starterSetup(
        channelsOnLoad: (_) => const [],
        refusalError: duplicateId,
        atId: _channel(
          id: generalId,
          name: 'general',
          archivedAt: DateTime.utc(2025),
        ),
      );
      final focusAfterArchived = await archivedAtId.recovery
          .ensureStarterChannels();
      expect(archivedAtId.created, ['general', 'welcome-everyone']);
      expect(archivedAtId.lookups, [generalId]);
      expect(archivedAtId.joined, isEmpty);
      expect(archivedAtId.loads(), 2);
      expect(focusAfterArchived, welcomeId);

      // The relay refuses a create at an archived channel's id with
      // `channel is archived`; that starter is skipped without a lookup.
      final refusedAsArchived = starterSetup(
        channelsOnLoad: (_) => const [],
        refusalError: 'invalid: channel is archived',
      );
      final focusAfterRefusal = await refusedAsArchived.recovery
          .ensureStarterChannels();
      expect(refusedAsArchived.created, ['general', 'welcome-everyone']);
      expect(refusedAsArchived.lookups, isEmpty);
      expect(refusedAsArchived.joined, isEmpty);
      expect(refusedAsArchived.loads(), 2);
      expect(focusAfterRefusal, welcomeId);

      // The scope fence also holds across the direct lookup, checked on the
      // last starter so the next starter's own fence cannot mask it.
      var lookedUp = false;
      final fencedLookup = starterSetup(
        channelsOnLoad: (_) => const [],
        refusalError: duplicateId,
        refused: const {'welcome-everyone'},
        atId: _channel(
          id: welcomeId,
          name: 'welcome-everyone',
          archivedAt: DateTime.utc(2025),
        ),
        isScopeCurrent: () => !lookedUp,
        onLookup: () => lookedUp = true,
      );
      await expectLater(
        fencedLookup.recovery.ensureStarterChannels(),
        throwsStateError,
      );
      expect(fencedLookup.lookups, [welcomeId]);

      // Any other duplicate stays recoverable: nothing at the id, or a
      // channel there that is not archived, may be a starter still settling.
      for (final atId in [null, _channel(id: generalId, name: 'general')]) {
        final settling = starterSetup(
          channelsOnLoad: (_) => const [],
          refusalError: duplicateId,
          atId: atId,
        );
        await expectLater(
          settling.recovery.ensureStarterChannels(),
          throwsException,
          reason: '$atId',
        );
        expect(settling.created, ['general']);
        expect(settling.lookups, [generalId]);
        expect(settling.joined, isEmpty);
      }

      // A taken name whose holder the directory does not show yet stays
      // recoverable: it may be a usable starter another client just created.
      final unseen = starterSetup(
        channelsOnLoad: (_) => const [],
        refusalError: refusal,
      );
      await expectLater(
        unseen.recovery.ensureStarterChannels(),
        throwsException,
      );
      expect(unseen.created, ['general']);
      expect(unseen.joined, isEmpty);
      expect(unseen.loads(), 2);
      expect(unseen.lookups, isEmpty);

      // An unlisted namesake does not hold the name, so it does not explain
      // the refusal either: the setup stays recoverable, and a retry joins
      // the real holder once it appears.
      final unlistedForum = _channel(
        id: 'unlisted-general',
        name: 'general',
        channelType: 'forum',
        isUnlisted: true,
        isMember: true,
      );
      final realGeneral = _channel(id: 'real-general', name: 'general');
      final unlisted = starterSetup(
        channelsOnLoad: (load) =>
            load <= 2 ? [unlistedForum] : [unlistedForum, realGeneral],
        refusalError: refusal,
      );
      await expectLater(
        unlisted.recovery.ensureStarterChannels(),
        throwsException,
      );
      expect(unlisted.created, ['general']);
      expect(unlisted.joined, isEmpty);
      final focusAfterRetry = await unlisted.recovery.ensureStarterChannels();
      expect(unlisted.joined, ['real-general']);
      expect(focusAfterRetry, welcomeId);

      // A usable open channel that appeared after the first load (lost race)
      // is joined rather than skipped.
      final raced = starterSetup(
        channelsOnLoad: (load) =>
            load == 1 ? const [] : [_channel(id: 'raced', name: 'general')],
        refusalError: refusal,
      );
      await raced.recovery.ensureStarterChannels();
      expect(raced.joined, ['raced']);

      // The scope fence still holds after a refusal and its reload.
      // The reload after the last starter's refusal must be the one that
      // observes the change, so only the post-reload fence can catch it.
      var current = true;
      var fencedLoads = 0;
      final fenced = starterSetup(
        channelsOnLoad: (_) {
          if (++fencedLoads > 2) current = false;
          return [unusableHolders.first.$1];
        },
        refusalError: refusal,
        refused: const {'general', 'welcome-everyone'},
        isScopeCurrent: () => current,
      );
      await expectLater(
        fenced.recovery.ensureStarterChannels(),
        throwsStateError,
      );
      expect(fenced.created, ['general', 'welcome-everyone']);
      expect(fenced.loads(), 3);
      expect(fenced.joined, isEmpty);
    },
  );

  test('starter setup surfaces an unavailable starter for recovery', () async {
    final joined = <String>[];
    final recovery = MobileInviteJoinRecovery(
      loadChannels: () async => [
        _channel(id: 'welcome-id', name: 'welcome-everyone'),
      ],
      fetchChannel: (_) async => fail('no direct lookup expected'),
      createChannel:
          ({
            required channelId,
            required name,
            required channelType,
            required visibility,
            description,
            ttlSeconds,
          }) async => throw Exception('relay unavailable'),
      joinChannel: (channelId) async => joined.add(channelId),
      relayHttpOrigin: 'https://relay.example.com',
    );

    await expectLater(
      recovery.ensureStarterChannels(),
      throwsA(isA<Exception>()),
    );

    expect(joined, isEmpty);
  });

  test(
    'starter setup recovery survives dismissal and container recreation',
    () async {
      final keys = nostr.Keys.generate();
      var generatedKeys = 0;
      var claimRequests = 0;
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final firstContainer = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          inviteKeyGeneratorProvider.overrideWithValue(() {
            generatedKeys++;
            return keys;
          }),
          inviteJoinRecoveryProvider.overrideWithValue(
            (_) =>
                _FakeInviteJoinRecovery(error: Exception('relay disconnected')),
          ),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              claimRequests++;
              return http.Response(
                jsonEncode({
                  'status': 'joined',
                  'host': 'relay.example.com',
                  'role': 'member',
                }),
                200,
              );
            }),
          ),
        ],
      );
      await firstContainer
          .read(inviteJoinProvider.notifier)
          .prepare(
            const InviteDeepLink(
              relayUrl: 'wss://relay.example.com',
              code: 'code',
            ),
          );
      await firstContainer.read(inviteJoinProvider.notifier).confirmJoin();

      final failed = firstContainer.read(inviteJoinProvider);
      expect(failed.status, InviteJoinStatus.error);
      expect(failed.isStarterSetupRecovery, isTrue);
      expect(generatedKeys, 1);
      expect(claimRequests, 1);
      expect((await storage.loadAll()).single.starterSetupIncomplete, isTrue);

      firstContainer.dispose();

      final secondContainer = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          inviteKeyGeneratorProvider.overrideWithValue(() {
            generatedKeys++;
            return nostr.Keys.generate();
          }),
          inviteJoinRecoveryProvider.overrideWithValue(
            (_) => _successfulRecovery(),
          ),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              claimRequests++;
              return http.Response('{}', 500);
            }),
          ),
        ],
      );
      addTearDown(secondContainer.dispose);

      await secondContainer
          .read(inviteJoinProvider.notifier)
          .prepare(
            const InviteDeepLink(
              relayUrl: 'wss://relay.example.com',
              code: 'code',
            ),
          );
      await secondContainer
          .read(inviteJoinProvider.notifier)
          .startStarterSetupRecovery();

      final recovered = secondContainer.read(inviteJoinProvider);
      expect(recovered.status, InviteJoinStatus.success);
      expect(recovered.focusChannelId, 'welcome-everyone-id');
      expect((await storage.loadAll()).single.starterSetupIncomplete, isFalse);
      expect(generatedKeys, 1);
      expect(claimRequests, 1);
    },
  );

  test(
    'starter recovery makes no replacement-tenant submission after a scope switch',
    () async {
      var isScopeCurrent = true;
      final firstJoinStarted = Completer<void>();
      final releaseFirstJoin = Completer<void>();
      final submitted = <String>[];
      final recovery = MobileInviteJoinRecovery(
        loadChannels: () async => [
          _channel(id: 'general-id', name: 'general'),
          _channel(id: 'welcome-id', name: 'welcome-everyone'),
        ],
        fetchChannel: (_) async => fail('no direct lookup expected'),
        createChannel:
            ({
              required channelId,
              required name,
              required channelType,
              required visibility,
              description,
              ttlSeconds,
            }) async => throw StateError('starters already exist'),
        joinChannel: (channelId) async {
          submitted.add(channelId);
          if (channelId == 'general-id') {
            firstJoinStarted.complete();
            await releaseFirstJoin.future;
          }
        },
        relayHttpOrigin: 'https://relay.example.com',
        isScopeCurrent: () => isScopeCurrent,
      );

      final setup = recovery.ensureStarterChannels();
      await firstJoinStarted.future;
      isScopeCurrent = false;
      releaseFirstJoin.complete();

      await expectLater(setup, throwsA(isA<StateError>()));
      expect(submitted, ['general-id']);
    },
  );

  test('join_policy_required requires a fresh link and cannot retry', () async {
    final keys = nostr.Keys.generate();
    var attempts = 0;
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        inviteKeyGeneratorProvider.overrideWithValue(() => keys),
        inviteJoinHttpClientProvider.overrideWithValue(
          http_testing.MockClient((request) async {
            attempts++;
            return http.Response(
              jsonEncode({'error': 'join_policy_required'}),
              403,
            );
          }),
        ),
      ],
    );
    addTearDown(container.dispose);

    await container
        .read(inviteJoinProvider.notifier)
        .prepare(
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'code',
            policyReceipt: 'expired.receipt',
          ),
        );
    await container.read(inviteJoinProvider.notifier).confirmJoin();

    final state = container.read(inviteJoinProvider);
    expect(state.status, InviteJoinStatus.error);
    expect(state.requiresFreshInvite, isTrue);
    expect(
      state.errorMessage,
      'Review this community’s join requirements in your browser, then return to Buzz.',
    );

    await container.read(inviteJoinProvider.notifier).confirmJoin();
    expect(attempts, 1);
  });

  for (final receipt in <String?>[null, 'expired.receipt']) {
    test(
      'policy recovery preserves identity and resumes claim ($receipt)',
      () async {
        final secure = FakeSecureStorage();
        final storage = CommunityStorage(secure: secure);
        final pubkeys = <String>[];
        final opened = <Uri>[];
        var launches = 0;
        final container = ProviderContainer(
          overrides: [
            communityStorageProvider.overrideWithValue(storage),
            communitySnapshotWriterProvider.overrideWithValue((_) async {}),
            authProvider.overrideWith(_RecordingAuthNotifier.new),
            inviteJoinRecoveryProvider.overrideWithValue(
              (_) => _successfulRecovery(),
            ),
            invitePolicyLauncherProvider.overrideWithValue((uri) async {
              opened.add(uri);
              return ++launches > 1;
            }),
            inviteJoinHttpClientProvider.overrideWithValue(
              http_testing.MockClient((request) async {
                final event =
                    jsonDecode(
                          utf8.decode(
                            base64Decode(
                              request.headers['Authorization']!.substring(
                                'Nostr '.length,
                              ),
                            ),
                          ),
                        )
                        as Map;
                pubkeys.add(event['pubkey'] as String);
                return (jsonDecode(request.body) as Map)['policy_receipt'] ==
                        'fresh.receipt'
                    ? http.Response(jsonEncode({'status': 'joined'}), 200)
                    : http.Response(
                        jsonEncode({'error': 'join_policy_required'}),
                        403,
                      );
              }),
            ),
          ],
        );
        addTearDown(container.dispose);
        final notifier = container.read(inviteJoinProvider.notifier);
        await notifier.prepare(
          InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'code',
            policyReceipt: receipt,
          ),
        );
        await notifier.confirmJoin();
        expect(
          container.read(inviteJoinProvider).requiresPolicyAcceptance,
          isTrue,
        );
        final pending = await secure.readAll();
        expect(await notifier.openPolicyAcceptance(), isFalse);
        expect(
          container.read(inviteJoinProvider).errorMessage,
          contains('Could not open the browser'),
        );
        expect(await notifier.openPolicyAcceptance(), isTrue);
        expect(
          opened,
          everyElement(Uri.parse('https://relay.example.com/invite/code')),
        );
        expect(await secure.readAll(), pending);
        expect(pubkeys, hasLength(1));
        await notifier.prepare(
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'code',
            policyReceipt: 'fresh.receipt',
          ),
        );
        await notifier.confirmJoin();
        expect(
          container.read(inviteJoinProvider).status,
          InviteJoinStatus.success,
        );
        expect(pubkeys, [pubkeys.first, pubkeys.first]);
      },
    );
  }

  test('invite_exhausted requires a fresh invite and cannot retry', () async {
    final keys = nostr.Keys.generate();
    var attempts = 0;
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        inviteKeyGeneratorProvider.overrideWithValue(() => keys),
        inviteJoinHttpClientProvider.overrideWithValue(
          http_testing.MockClient((request) async {
            attempts++;
            return http.Response(
              jsonEncode({'error': 'invite_exhausted'}),
              403,
            );
          }),
        ),
      ],
    );
    addTearDown(container.dispose);

    await container
        .read(inviteJoinProvider.notifier)
        .prepare(
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'v2.exhausted-secret',
          ),
        );
    await container.read(inviteJoinProvider.notifier).confirmJoin();

    final state = container.read(inviteJoinProvider);
    expect(state.status, InviteJoinStatus.error);
    expect(state.requiresFreshInvite, isTrue);
    expect(
      state.errorMessage,
      'This invite has reached its use limit. Ask for a new invite.',
    );

    await container.read(inviteJoinProvider.notifier).confirmJoin();
    expect(attempts, 1);
  });

  for (final (sentinel, guidance) in [
    (
      'invite_restricted',
      'This account, or the account that owns it, is restricted in this community. Contact a community administrator. If the restriction is temporary, try again after it ends.',
    ),
    (
      'invite_owner_mismatch',
      'This account does not match its recorded owner. Use the correct account or owner identity to join this community.',
    ),
  ]) {
    test('$sentinel explains recovery without replacing the invite', () async {
      final keys = nostr.Keys.generate();
      var attempts = 0;
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final auth = _RecordingAuthNotifier();
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          authProvider.overrideWith(() => auth),
          inviteKeyGeneratorProvider.overrideWithValue(() => keys),
          inviteJoinRecoveryProvider.overrideWithValue(
            (_) => _successfulRecovery(),
          ),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              attempts++;
              expect(jsonDecode(request.body)['code'], 'same-code');
              return attempts == 1
                  ? http.Response(jsonEncode({'error': sentinel}), 403)
                  : http.Response(jsonEncode({'status': 'joined'}), 200);
            }),
          ),
        ],
      );
      addTearDown(container.dispose);
      final notifier = container.read(inviteJoinProvider.notifier);
      await notifier.prepare(
        const InviteDeepLink(
          relayUrl: 'wss://relay.example.com',
          code: 'same-code',
        ),
      );
      await notifier.confirmJoin();

      final denied = container.read(inviteJoinProvider);
      expect(denied.status, InviteJoinStatus.error);
      expect(denied.errorMessage, guidance);
      expect(denied.requiresFreshInvite, isFalse);
      expect(attempts, 1);
      expect(auth.authenticatedCommunities, isEmpty);
      expect(await storage.loadAll(), isEmpty);

      // Model an explicit retry after the restriction or identity is corrected.
      await notifier.confirmJoin();
      expect(
        container.read(inviteJoinProvider).status,
        InviteJoinStatus.success,
      );
      expect(attempts, 2);
    });
  }

  test(
    'committed claim with lost response retries the same key after restart',
    () async {
      var attempts = 0;
      String? claimedPubkey;
      String? lastClaimPubkey;
      final bodies = <String>[];
      final secure = FakeSecureStorage();
      final storage = CommunityStorage(secure: secure);
      final auth = _RecordingAuthNotifier();
      ProviderContainer open() => ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          communityPushLeaseRevocationEnqueuerProvider.overrideWithValue(
            (_) async => false,
          ),
          authProvider.overrideWith(() => auth),
          // Keep the production fresh-key generator: a fixed fake masks key loss.
          inviteJoinRecoveryProvider.overrideWithValue(
            (_) => _successfulRecovery(),
          ),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              attempts++;
              bodies.add(request.body);
              final event =
                  jsonDecode(
                        utf8.decode(
                          base64Decode(
                            request.headers['Authorization']!.substring(
                              'Nostr '.length,
                            ),
                          ),
                        ),
                      )
                      as Map;
              final pubkey = event['pubkey'] as String;
              lastClaimPubkey = pubkey;
              if (attempts == 1) {
                claimedPubkey = pubkey;
                throw http.ClientException('Response lost after commit');
              }
              if (pubkey != claimedPubkey) {
                return http.Response(
                  jsonEncode({'error': 'invite exhausted'}),
                  403,
                );
              }
              return http.Response(
                jsonEncode({
                  'status': 'already_member',
                  'host': 'relay.example.com',
                  'role': 'member',
                }),
                200,
              );
            }),
          ),
        ],
      );
      const invite = InviteDeepLink(
        relayUrl: 'wss://relay.example.com',
        code: 'code',
        policyReceipt: 'receipt.value',
      );
      var container = open();
      await container.read(inviteJoinProvider.notifier).prepare(invite);
      await container.read(inviteJoinProvider.notifier).confirmJoin();
      expect(container.read(inviteJoinProvider).status, InviteJoinStatus.error);
      expect(await storage.loadAll(), isEmpty);
      container.dispose();
      container = open();
      addTearDown(container.dispose);
      await container.read(inviteJoinProvider.notifier).prepare(invite);
      await container.read(inviteJoinProvider.notifier).confirmJoin();
      expect(
        container.read(inviteJoinProvider).status,
        InviteJoinStatus.success,
      );
      expect(attempts, 2);
      expect(
        bodies,
        everyElement(
          jsonEncode({'code': 'code', 'policy_receipt': 'receipt.value'}),
        ),
      );
      expect((await storage.loadAll()).single.pubkey, claimedPubkey);
      expect(
        (await secure.readAll()).keys.where(
          (key) => key.startsWith('buzz_pending_invite_'),
        ),
        isEmpty,
      );
      final saved = (await storage.loadAll()).single;
      // A crash after saving the community can leave the journal behind.
      await storage.pendingInviteIdentity(
        invite.relayUrl,
        invite.code,
        () => saved.nsec!,
      );
      await container.read(inviteJoinProvider.notifier).prepare(invite);
      expect(
        (await secure.readAll()).values.where((value) => value == saved.nsec),
        isEmpty,
      );

      // Also cover a destination saved after the confirmation page was opened.
      await storage.saveAll([]);
      container.invalidate(communityListProvider);
      await container.read(inviteJoinProvider.notifier).prepare(invite);
      await storage.saveAll([saved]);
      container.invalidate(communityListProvider);
      await storage.pendingInviteIdentity(
        invite.relayUrl,
        invite.code,
        () => saved.nsec!,
      );
      await container.read(inviteJoinProvider.notifier).confirmJoin();
      expect(
        (await secure.readAll()).values.where((value) => value == saved.nsec),
        isEmpty,
      );

      // Any matching journal, including a different invite, must be retired on
      // sign-out. Unrelated pending identities must survive.
      await storage.pendingInviteIdentity(
        invite.relayUrl,
        invite.code,
        () => saved.nsec!,
      );
      await storage.pendingInviteIdentity(
        invite.relayUrl,
        'another-code',
        () => saved.nsec!,
      );
      final unrelated = nostr.Keys.generate().nsec;
      await storage.pendingInviteIdentity(
        'wss://other.example',
        'other-code',
        () => unrelated,
      );
      secure.failPendingInviteDeletes = true;
      await expectLater(
        container
            .read(communityListProvider.notifier)
            .removeCommunity(saved.id),
        throwsStateError,
      );
      expect((await storage.loadAll()).single.nsec, saved.nsec);
      secure.failPendingInviteDeletes = false;
      await container
          .read(communityListProvider.notifier)
          .removeCommunity(saved.id);
      expect(await storage.loadAll(), isEmpty);
      expect((await secure.readAll()).values, isNot(contains(saved.nsec)));
      expect((await secure.readAll()).values, contains(unrelated));
      await container.read(inviteJoinProvider.notifier).prepare(invite);
      await container.read(inviteJoinProvider.notifier).confirmJoin();
      expect(attempts, 3);
      expect(lastClaimPubkey, isNot(claimedPubkey));
      expect(container.read(inviteJoinProvider).status, InviteJoinStatus.error);
      expect(await storage.loadAll(), isEmpty);
    },
  );

  test('builds a fresh recovery with the second community identity', () async {
    final firstKeys = nostr.Keys.generate();
    final secondKeys = nostr.Keys.generate();
    final scopes = <InviteJoinRecoveryScope>[];
    final recoveries = <InviteJoinRecoveryScope>[];
    var nextKeys = 0;
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(
          CommunityStorage(secure: FakeSecureStorage()),
        ),
        authProvider.overrideWith(_RecordingAuthNotifier.new),
        inviteKeyGeneratorProvider.overrideWithValue(() {
          final keys = nextKeys == 0 ? firstKeys : secondKeys;
          nextKeys++;
          return keys;
        }),
        inviteJoinRecoveryProvider.overrideWithValue((scope) {
          scopes.add(scope);
          return _RecordingInviteJoinRecovery(() async {
            recoveries.add(scope);
            return 'welcome-everyone-id';
          });
        }),
        inviteJoinHttpClientProvider.overrideWithValue(
          http_testing.MockClient(
            (request) async => http.Response(
              jsonEncode({
                'status': 'joined',
                'host': request.url.host,
                'role': 'member',
              }),
              200,
            ),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);

    for (final invite in const [
      InviteDeepLink(relayUrl: 'wss://first.example.com', code: 'first'),
      InviteDeepLink(relayUrl: 'wss://second.example.com', code: 'second'),
    ]) {
      await container.read(inviteJoinProvider.notifier).prepare(invite);
      await container.read(inviteJoinProvider.notifier).confirmJoin();
      expect(
        container.read(inviteJoinProvider).status,
        InviteJoinStatus.success,
      );
    }

    expect(scopes.map((scope) => scope.relayHttpOrigin), [
      'https://first.example.com',
      'https://second.example.com',
    ]);
    expect(scopes.map((scope) => scope.nsec), [
      firstKeys.nsec,
      secondKeys.nsec,
    ]);
    expect(recoveries, hasLength(2));
    expect(identical(recoveries[0], scopes[0]), isTrue);
    expect(identical(recoveries[1], scopes[1]), isTrue);
    expect(identical(recoveries[1], scopes[0]), isFalse);
  });
}

InviteJoinRecovery _successfulRecovery() =>
    const _FakeInviteJoinRecovery(focusChannelId: 'welcome-everyone-id');

class _FakeInviteJoinRecovery implements InviteJoinRecovery {
  final String? focusChannelId;
  final Object? error;

  const _FakeInviteJoinRecovery({this.focusChannelId, this.error});

  @override
  Future<String?> ensureStarterChannels() async {
    if (error case final failure?) throw failure;
    return focusChannelId;
  }
}

class _RecordingInviteJoinRecovery implements InviteJoinRecovery {
  const _RecordingInviteJoinRecovery(this._ensure);

  final Future<String?> Function() _ensure;

  @override
  Future<String?> ensureStarterChannels() => _ensure();
}

Channel _channel({
  required String id,
  required String name,
  String visibility = 'open',
  bool isMember = false,
  String channelType = 'stream',
  DateTime? archivedAt,
  bool isUnlisted = false,
}) => Channel(
  id: id,
  name: name,
  channelType: channelType,
  archivedAt: archivedAt,
  isUnlisted: isUnlisted,
  visibility: visibility,
  description: '',
  createdBy: 'me',
  createdAt: DateTime.utc(2026),
  memberCount: isMember ? 1 : 0,
  isMember: isMember,
);

class _RecordingAuthNotifier extends AuthNotifier {
  final List<Community> authenticatedCommunities = [];

  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  @override
  Future<void> authenticateWithCommunity(
    Community community, {
    bool Function()? shouldContinue,
    bool activate = true,
  }) async {
    if (shouldContinue != null && !shouldContinue()) return;
    final storage = ref.read(communityStorageProvider);
    await storage.save(community);
    await storage.saveActiveId(community.id);
    ref.invalidate(communityListProvider);
    ref.invalidate(activeCommunityProvider);
    authenticatedCommunities.add(community);
    state = AsyncData(
      AuthState(status: AuthStatus.authenticated, community: community),
    );
  }
}
