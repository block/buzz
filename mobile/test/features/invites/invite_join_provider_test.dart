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

  test(
    'policy-gated invite accepts policy and retries claim with receipt',
    () async {
      final keys = nostr.Keys.generate();
      final requests = <http.Request>[];
      var claimAttempts = 0;
      var generatedKeys = 0;
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final auth = _RecordingAuthNotifier();
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          authProvider.overrideWith(() => auth),
          inviteKeyGeneratorProvider.overrideWithValue(() {
            generatedKeys++;
            return keys;
          }),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              requests.add(request);
              if (request.url.path == '/api/join-policy') {
                return http.Response(
                  jsonEncode({
                    'policy': {
                      'terms_markdown': '# Be kind',
                      'privacy_markdown': 'We retain your messages.',
                      'age_attestation_required': true,
                      'version': 'policy-v1',
                    },
                  }),
                  200,
                );
              }
              if (request.url.path == '/api/invites/accept-policy') {
                return http.Response(
                  jsonEncode({'receipt': 'new.receipt'}),
                  200,
                );
              }
              claimAttempts++;
              if (claimAttempts == 1) {
                return http.Response(
                  jsonEncode({'error': 'join_policy_required'}),
                  403,
                );
              }
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
      addTearDown(container.dispose);

      await container
          .read(inviteJoinProvider.notifier)
          .prepare(
            const InviteDeepLink(
              relayUrl: 'wss://relay.example.com',
              code: 'code',
            ),
          );
      await container.read(inviteJoinProvider.notifier).confirmJoin();

      var state = container.read(inviteJoinProvider);
      expect(state.status, InviteJoinStatus.reviewingPolicy);
      expect(state.policy?.termsMarkdown, '# Be kind');
      expect(state.policy?.privacyMarkdown, 'We retain your messages.');
      expect(state.ageConfirmed, isFalse);
      expect(state.agreementConfirmed, isFalse);
      expect(state.requiresFreshInvite, isFalse);

      await container.read(inviteJoinProvider.notifier).acceptPolicy();
      state = container.read(inviteJoinProvider);
      expect(state.status, InviteJoinStatus.reviewingPolicy);
      expect(state.errorMessage, 'Confirm that you are at least 18 years old.');
      expect(requests, hasLength(2));

      final notifier = container.read(inviteJoinProvider.notifier);
      notifier.setAgeConfirmed(true);
      notifier.setAgreementConfirmed(true);
      await notifier.acceptPolicy();

      state = container.read(inviteJoinProvider);
      expect(state.status, InviteJoinStatus.success);
      expect(claimAttempts, 2);
      expect(generatedKeys, 1);
      expect(requests.map((request) => request.url.path), [
        '/api/invites/claim',
        '/api/join-policy',
        '/api/invites/accept-policy',
        '/api/invites/claim',
      ]);
      expect(
        requests[2].body,
        jsonEncode({
          'code': 'code',
          'policy_version': 'policy-v1',
          'age_confirmed': true,
        }),
      );
      expect(
        requests[3].body,
        jsonEncode({'code': 'code', 'policy_receipt': 'new.receipt'}),
      );
      expect(auth.authenticatedCommunities.single.pubkey, keys.public);
    },
  );

  test('declining join policy stops without accepting or joining', () async {
    final keys = nostr.Keys.generate();
    final requests = <http.Request>[];
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final auth = _RecordingAuthNotifier();
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        authProvider.overrideWith(() => auth),
        inviteKeyGeneratorProvider.overrideWithValue(() => keys),
        inviteJoinHttpClientProvider.overrideWithValue(
          http_testing.MockClient((request) async {
            requests.add(request);
            if (request.url.path == '/api/join-policy') {
              return http.Response(
                jsonEncode({
                  'policy': {
                    'terms_markdown': 'Community terms',
                    'privacy_markdown': null,
                    'age_attestation_required': true,
                    'version': 'policy-v1',
                  },
                }),
                200,
              );
            }
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
          ),
        );
    await container.read(inviteJoinProvider.notifier).confirmJoin();
    container.read(inviteJoinProvider.notifier).declinePolicy();

    final state = container.read(inviteJoinProvider);
    expect(state.status, InviteJoinStatus.declined);
    expect(
      state.errorMessage,
      'You declined this community\'s join policy. You were not joined.',
    );
    expect(requests, hasLength(2));
    expect(auth.authenticatedCommunities, isEmpty);
  });

  test('accept-policy failure stays on policy review with an error', () async {
    final keys = nostr.Keys.generate();
    final requests = <http.Request>[];
    final storage = CommunityStorage(secure: FakeSecureStorage());
    final auth = _RecordingAuthNotifier();
    final container = ProviderContainer(
      overrides: [
        communityStorageProvider.overrideWithValue(storage),
        authProvider.overrideWith(() => auth),
        inviteKeyGeneratorProvider.overrideWithValue(() => keys),
        inviteJoinHttpClientProvider.overrideWithValue(
          http_testing.MockClient((request) async {
            requests.add(request);
            if (request.url.path == '/api/join-policy') {
              return http.Response(
                jsonEncode({
                  'policy': {
                    'terms_markdown': 'Community terms',
                    'privacy_markdown': null,
                    'age_attestation_required': true,
                    'version': 'policy-v1',
                  },
                }),
                200,
              );
            }
            if (request.url.path == '/api/invites/accept-policy') {
              return http.Response(
                jsonEncode({'error': 'temporary_policy_failure'}),
                503,
              );
            }
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
          ),
        );
    await container.read(inviteJoinProvider.notifier).confirmJoin();
    final notifier = container.read(inviteJoinProvider.notifier);
    notifier.setAgeConfirmed(true);
    notifier.setAgreementConfirmed(true);
    await notifier.acceptPolicy();

    final state = container.read(inviteJoinProvider);
    expect(state.status, InviteJoinStatus.reviewingPolicy);
    expect(
      state.errorMessage,
      'Could not accept this community\'s join policy: temporary_policy_failure',
    );
    expect(requests, hasLength(3));
    expect(auth.authenticatedCommunities, isEmpty);
  });

  test(
    'invite carrying a policy receipt claims without policy handshake',
    () async {
      final keys = nostr.Keys.generate();
      final requests = <http.Request>[];
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final auth = _RecordingAuthNotifier();
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          authProvider.overrideWith(() => auth),
          inviteKeyGeneratorProvider.overrideWithValue(() => keys),
          inviteJoinHttpClientProvider.overrideWithValue(
            http_testing.MockClient((request) async {
              requests.add(request);
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
      addTearDown(container.dispose);

      await container
          .read(inviteJoinProvider.notifier)
          .prepare(
            const InviteDeepLink(
              relayUrl: 'wss://relay.example.com',
              code: 'code',
              policyReceipt: 'existing.receipt',
            ),
          );
      await container.read(inviteJoinProvider.notifier).confirmJoin();

      expect(
        container.read(inviteJoinProvider).status,
        InviteJoinStatus.success,
      );
      expect(requests, hasLength(1));
      expect(requests.single.url.path, '/api/invites/claim');
      expect(
        requests.single.body,
        jsonEncode({'code': 'code', 'policy_receipt': 'existing.receipt'}),
      );
      expect(auth.authenticatedCommunities, hasLength(1));
    },
  );

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

  test('failed claim can be retried and preserves policy receipt', () async {
    final keys = nostr.Keys.generate();
    var attempts = 0;
    final bodies = <String>[];
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
            bodies.add(request.body);
            if (attempts == 1) {
              return http.Response(jsonEncode({'error': 'temporary'}), 503);
            }
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
    addTearDown(container.dispose);

    await container
        .read(inviteJoinProvider.notifier)
        .prepare(
          const InviteDeepLink(
            relayUrl: 'wss://relay.example.com',
            code: 'code',
            policyReceipt: 'receipt.value',
          ),
        );
    await container.read(inviteJoinProvider.notifier).confirmJoin();
    expect(container.read(inviteJoinProvider).status, InviteJoinStatus.error);

    await container.read(inviteJoinProvider.notifier).confirmJoin();

    expect(container.read(inviteJoinProvider).status, InviteJoinStatus.success);
    expect(attempts, 2);
    expect(
      bodies,
      everyElement(
        jsonEncode({'code': 'code', 'policy_receipt': 'receipt.value'}),
      ),
    );
    expect(auth.authenticatedCommunities, hasLength(1));
  });
}

class _RecordingAuthNotifier extends AuthNotifier {
  final List<Community> authenticatedCommunities = [];

  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);

  @override
  Future<void> authenticateWithCommunity(Community community) async {
    authenticatedCommunities.add(community);
    state = AsyncData(
      AuthState(status: AuthStatus.authenticated, community: community),
    );
  }
}
