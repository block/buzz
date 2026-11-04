import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

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

  test('each claimed community gets its own identity', () async {
    final firstKeys = nostr.Keys.generate();
    final secondKeys = nostr.Keys.generate();
    final auth = _RecordingAuthNotifier();
    var nextKeys = 0;
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(
          CommunityStorage(secure: FakeSecureStorage()),
        ),
        authProvider.overrideWith(() => auth),
        inviteKeyGeneratorProvider.overrideWithValue(() {
          final keys = nextKeys == 0 ? firstKeys : secondKeys;
          nextKeys++;
          return keys;
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

    expect(auth.authenticatedCommunities.map((community) => community.nsec), [
      firstKeys.nsec,
      secondKeys.nsec,
    ]);
    expect(
      auth.authenticatedCommunities.map((community) => community.relayUrl),
      ['wss://first.example.com', 'wss://second.example.com'],
    );
  });
}

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
