import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/features/pairing/pairing_provider.dart';
import 'dart:io';
import 'dart:convert';
import 'package:flutter/services.dart';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/invites/invite_join_provider.dart';
import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/hosted_app_gate.dart';
import 'package:buzz/shared/hosted/hosted_flow_provider.dart';
import 'package:buzz/onboarding_app_entry.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/deeplink/pending_deep_link_provider.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/browser_auth.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_flow.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/relay/relay_socket.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'widget_helpers.dart';

/// Synthetic REST/relay boundaries with the production flow factory, account,
/// identity, admission, auth and storage. No admission/persistence is simulated.
class HostedJourneyFixture {
  final keys = nostr.Keys('1'.padLeft(64, '0'));
  final secondKeys = nostr.Keys('2'.padLeft(64, '0'));
  AccountIdentityStore get store => AccountIdentityStore(subject: subject);
  final storage = CommunityStorage();
  final identityStorage = JourneyIdentityStorage();
  final requests = <http.Request>[];
  Future<void> Function()? beforeBrowserReturn;
  Future<void> Function()? beforeStatusReturn;
  Future<void> Function()? beforeEnrollmentReturn;
  Future<void> Function()? beforeUploadReturn;
  final probes = <Community>[];
  String? refusedSelfHostedPubkey;
  final profileWrites = <String>[];
  final profileAvatars = <String?>[];
  final recoveryScopes = <InviteJoinRecoveryScope>[];
  final rows = <Map<String, dynamic>>[];
  final cloudKeys = <nostr.Keys>[];
  HostedFlow? activeFlow;
  HostedFlow get flow => activeFlow!;
  late ProviderContainer container;
  String? bound;
  int starterFailures = 0;
  final policyResponses = <http.Response>[];
  int browserStarts = 0;
  int secondRelayFailures = 0;
  DateTime assertionNow = DateTime.now();
  bool relayAuthEnabled = false;
  bool expiredSession = false;
  bool denyAssertion = false;
  bool refuseUpgrade = false;
  String subject = 'journey-owner';
  Future<void> Function()? beforeAssertionReturn;
  final socketHeaders = <Map<String, String>>[];
  bool offerProfile = true;
  bool profileUnavailable = false;
  bool rejectList = false;
  int saveFailures = 0;
  final knownSaved = <String>{};
  bool missingList = false;
  bool loseCreateResponse = false;
  bool loseClaimResponse = false;
  bool inviteExpired = false;
  String? claimError;
  final owned = <Map<String, dynamic>>[];
  final expiry = DateTime.now()
      .toUtc()
      .add(const Duration(minutes: 5))
      .toIso8601String();

  Map<String, dynamic> row(String name) => {
    'id': name,
    'name': name,
    'normalized_host': '$name.communities.buzz.xyz',
  };

  Future<void> initialize({
    bool localIdentity = true,
    bool cloudChoice = false,
    bool signedIn = true,
  }) async {
    FlutterSecureStorage.setMockInitialValues({});
    PendingDeepLinkNotifier.debugUriStreamOverride = const Stream<Uri>.empty();
    bound = cloudChoice ? null : keys.public;
    if (localIdentity) {
      await store.create(keys.nsec);
      await store.confirmBackup(
        BackupReceipt(
          backend: BuilderlabAccountClient.origin.origin,
          subject: 'journey-owner',
          pubkey: keys.public,
          backupId: 'backup-1',
        ),
      );
    }
    // A real exchange writes the credential used by both the flow and the
    // independent account verifier constructed during durable admission.
    if (!signedIn) return;
    final account = BuilderlabAccountClient();
    await account.exchange(code: 'synthetic-code', verifier: 'v' * 43);
    account.dispose();
  }

  http.Client client() => MockClient((request) async {
    requests.add(request);
    final path = request.url.path;
    Object result;
    if (path.endsWith('/exchange')) {
      expiredSession = false;
      result = {
        'session_credential': 'synthetic-session',
        'expires_at': expiry,
      };
    } else if (path.endsWith('/me')) {
      if (expiredSession) return http.Response('{}', 401);
      result = {'subject': subject, 'expires_at': expiry};
    } else if (path.endsWith('/identity/assertions')) {
      await beforeAssertionReturn?.call();
      if (expiredSession) return http.Response('{}', 401);
      if (denyAssertion) return http.Response('{}', 403);
      final body = jsonDecode(request.body) as Map;
      if (bound != null && body['nostr_pubkey'] != bound) {
        return http.Response('{}', 403);
      }
      bound ??= body['nostr_pubkey'] as String;
      result = {
        'assertion': 'synthetic-assertion',
        'nostr_pubkey': bound,
        'expires_at':
            assertionNow
                .add(const Duration(minutes: 5))
                .millisecondsSinceEpoch ~/
            1000,
      };
    } else if (path == '/auth-transport') {
      return http.Response(
        '',
        302,
        headers: {'location': 'https://outside.example'},
      );
    } else if (path == '/query' || path == '/profiles/search') {
      if (relayAuthEnabled &&
          HostedEnvironment.requiresAuth(request.url.host)) {
        expect(
          request.headers['Nostr-Federated-Identity'],
          'Bearer synthetic-assertion',
        );
        expect(request.headers.containsKey('X-BB-Session-Credential'), isFalse);
      }
      result = <Object>[];
    } else if (path.endsWith('/current')) {
      if (expiredSession) return http.Response('{}', 401);
      result = {
        'identity': bound == null ? null : {'pubkey_hex': bound},
      };
    } else if (path.endsWith('/status')) {
      await beforeStatusReturn?.call();
      result = {
        'current_nostr_pubkey': bound,
        'backup_nostr_pubkeys': cloudKeys.map((k) => k.public).toList(),
      };
    } else if (path.endsWith('/recovery')) {
      result = {
        'backups': [
          for (final k in cloudKeys)
            {
              'backup_id': 'backup-${k.public}',
              'nostr_pubkey': k.public,
              'secret_key': base64Url
                  .encode([
                    for (var i = 0; i < 64; i += 2)
                      int.parse(k.secret.substring(i, i + 2), radix: 16),
                  ])
                  .replaceAll('=', ''),
            },
        ],
        'failed_backup_ids': <String>[],
      };
    } else if (path.endsWith('/challenge')) {
      result = {
        'challenge_id': '12345678-1234-1234-1234-123456789abc',
        'nonce': 'a' * 43,
        'verification_code': '123456',
        'origin': BuilderlabAccountClient.origin.toString(),
        'expires_at': expiry,
      };
    } else if (path.endsWith('/verify')) {
      final event =
          jsonDecode(jsonDecode(request.body)['signed_payload'] as String)
              as Map;
      bound = event['pubkey'] as String;
      result = {
        'identity': {'pubkey_hex': bound},
      };
    } else if (path.endsWith('/enrollment')) {
      await beforeEnrollmentReturn?.call();
      result = {
        'version': 1,
        'suite': {'kem_id': 16, 'kdf_id': 1, 'aead_id': 2},
        'recipient_key_id': 'test-key',
        'service_namespace': 'buzz',
        'owner_id': 'journey-owner',
        'backup_id': '12345678-1234-1234-1234-123456789abc',
        'nostr_pubkey': bound,
        'recipient_public_key': base64Url
            .encode([4, ...List.filled(64, 1)])
            .replaceAll('=', ''),
      };
    } else if (path.endsWith('/nsec-backups')) {
      await beforeUploadReturn?.call();
      result = {
        'stored': true,
        'backup_id': jsonDecode(request.body)['backup_id'],
      };
    } else if (path.endsWith('/v1/buzz/communities')) {
      final name = jsonDecode(request.body)['name'] as String;
      final created =
          owned
              .where(
                (row) => row['name'] == name && row['owner_pubkey'] == bound,
              )
              .firstOrNull ??
          {...row(name), 'owner_pubkey': bound};
      if (!owned.contains(created)) owned.add(created);
      if (loseCreateResponse) {
        loseCreateResponse = false;
        throw AccountFailure.unavailable;
      }
      result = {'community': created};
    } else if (path.endsWith('/known-communities/list')) {
      if (rejectList) return http.Response('{}', 503);
      result = {
        if (rows.isNotEmpty)
          'communities': [
            for (final row in rows)
              {'community_url': 'wss://${row['normalized_host']}'},
          ],
      };
    } else if (path.endsWith('/known-communities/add')) {
      expect(request.method, 'POST');
      final url = jsonDecode(request.body)['community_url'] as String;
      knownSaved.add(url);
      if (saveFailures-- > 0) return http.Response('{}', 503);
      result = {
        'community': {'community_url': url},
      };
    } else if (path.endsWith('/communities/joined')) {
      if (rejectList) return http.Response('{}', 503);
      if (missingList) return http.Response('{}', 404);
      result = {'communities': rows};
    } else if (path == '/api/join-policy') {
      if (policyResponses.isNotEmpty) return policyResponses.removeAt(0);
      result = {
        'policy': {
          'version': request.url.host,
          'age_attestation_required': true,
          'terms_markdown': 'Be kind to this community.',
          'privacy_markdown': 'Your public profile is public.',
        },
      };
    } else if (path == '/api/invites/accept-policy') {
      if (inviteExpired) {
        return http.Response(jsonEncode({'error': 'invite_expired'}), 410);
      }
      result = {'receipt': 'synthetic-policy-receipt'};
    } else if (path == '/api/invites/claim') {
      if (claimError != null) {
        return http.Response(jsonEncode({'error': claimError}), 403);
      }
      if (loseClaimResponse) {
        loseClaimResponse = false;
        rows.add(row(request.url.host.split('.').first));
        throw AccountFailure.unavailable;
      }
      result = {'host': request.url.host};
    } else {
      throw StateError('Unexpected journey request: ${request.method} $path');
    }
    return http.Response(
      jsonEncode(result),
      200,
      headers: {'content-type': 'application/json'},
    );
  });

  HostedFlow createFlow(Ref ref) => activeFlow = createHostedFlow(
    ref,
    browserAuth: _Browser(() async {
      browserStarts++;
      await beforeBrowserReturn?.call();
    }),
    probe: (community) async {
      probes.add(community);
      if (Uri.parse(community.relayUrl).host.startsWith('two.') &&
          secondRelayFailures-- > 0) {
        throw AccountFailure.unavailable;
      }
    },
  );

  Future<void> mount(
    WidgetTester tester, {
    bool openEntry = true,
    PairingCredentialValidator? pairingValidator,
  }) async {
    await tester.binding.setSurfaceSize(const Size(440, 1100));
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      const MethodChannel('buzz/hosted_hpke'),
      (_) async => {
        'enc': Uint8List.fromList([4, ...List.filled(64, 2)]),
        'ciphertext': Uint8List(48),
      },
    );
    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [
          hostedFlowProvider.overrideWith(createFlow),
          if (pairingValidator != null)
            pairingProvider.overrideWith(
              () => PairingNotifier(credentialValidator: pairingValidator),
            ),
          if (relayAuthEnabled) ...[
            hostedRelayAuthProvider.overrideWith((ref) {
              final auth = HostedRelayAuth(
                ref.watch(builderlabAccountProvider),
                enabled: true,
                now: () => assertionNow,
              );
              ref.onDispose(auth.dispose);
              return auth;
            }),
            relaySessionProvider.overrideWith(
              () => RelaySessionNotifier(
                socketFactory:
                    ({
                      required wsUrl,
                      required nsec,
                      required onMessage,
                      required onConnected,
                      required onDisconnected,
                    }) => _RelaySocket(
                      this,
                      wsUrl: wsUrl,
                      nsec: nsec,
                      onMessage: onMessage,
                      onConnected: onConnected,
                      onDisconnected: onDisconnected,
                    ),
              ),
            ),
          ],
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          channelsProvider.overrideWith(_NoChannels.new),
          profileProvider.overrideWith(() => _Profile(this)),
          inviteJoinRecoveryProvider.overrideWithValue(
            (scope) => _Recovery(this, scope),
          ),
          selfHostedEntryProvider.overrideWithValue(
            SelfHostedEntry(
              accountStore: AccountIdentityStore(storage: identityStorage),
              probe: (community) async {
                probes.add(community);
                if (community.pubkey == refusedSelfHostedPubkey) {
                  throw StateError('Community refused entry');
                }
              },
            ),
          ),
        ],
        child: HostedAppGate(
          authenticated: true,
          child: Builder(
            builder: (context) => Center(
              child: FilledButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) =>
                        const OnboardingAppEntry(addingCommunity: true),
                  ),
                ),
                child: const Text('Community home'),
              ),
            ),
          ),
        ),
      ),
    );
    await settle(tester);
    container = ProviderScope.containerOf(
      tester.element(find.byType(HostedAppGate)),
    );
    await container.read(authProvider.future);
    if (openEntry) {
      await tap(tester, 'Community home');
      expect(find.byType(OnboardingAppEntry), findsOneWidget);
    }
  }

  /// Recreate app owners while retaining secure storage and remote fixtures.
  Future<void> restart(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    await Future<void>.delayed(Duration.zero);
    await tester.pump();
    await mount(tester, openEntry: false);
  }

  // HTTP response streams use real event-loop turns. Yield between frames so
  // spinner animation cannot exhaust the fake clock before those turns run.
  Future<void> settle(WidgetTester tester) async {
    for (var i = 0; i < 60; i++) {
      await Future<void>.delayed(Duration.zero);
      await tester.pump(const Duration(milliseconds: 50));
      if (i >= 15 && activeFlow?.stage != HostedFlowStage.working) return;
    }
    throw StateError(
      'Journey did not settle: ${flow.stage} / ${flow.problem}; requests: ${requests.map((r) => r.url.path).join(',')}',
    );
  }

  Future<void> tap(WidgetTester tester, String label) async {
    await tester.pump();
    await tester.tap(find.text(label).hitTestable());
    await settle(tester);
  }

  Future<void> link(
    WidgetTester tester,
    String name, {
    bool invite = true,
  }) async {
    container
        .read(pendingDeepLinkProvider.notifier)
        .open(
          Uri(
            scheme: 'buzz',
            host: invite ? 'join' : 'add',
            queryParameters: {
              'relay': 'wss://$name.communities.buzz.xyz',
              if (invite) 'code': 'invite-$name',
              if (!invite) 'name': name,
            },
          ),
        );
    await settle(tester);
  }

  List<http.Request> sent(String path) =>
      requests.where((r) => r.url.path.endsWith(path)).toList();

  Future<void> dispose(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    await Future<void>.delayed(Duration.zero);
    await tester.pump();
    PendingDeepLinkNotifier.debugUriStreamOverride = null;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      const MethodChannel('buzz/hosted_hpke'),
      null,
    );
    await tester.binding.setSurfaceSize(null);
  }
}

/// Keeps direct reads working when an unrelated secure-storage entry breaks
/// enumeration, as can happen on Android after the identity picker is shown.
class JourneyIdentityStorage extends FlutterSecureStorage {
  bool failEnumeration = false;

  @override
  Future<Map<String, String>> readAll({
    AppleOptions? iOptions,
    AndroidOptions? aOptions,
    LinuxOptions? lOptions,
    WebOptions? webOptions,
    AppleOptions? mOptions,
    WindowsOptions? wOptions,
  }) async {
    if (failEnumeration) throw StateError('unrelated entry cannot decrypt');
    return super.readAll();
  }
}

class _NoChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

// Replace relay metadata transport, not the production profile-offer logic.
class _Profile extends ProfileNotifier {
  _Profile(this.fixture);
  final HostedJourneyFixture fixture;
  @override
  Future<UserProfile?> build() async {
    if (fixture.profileUnavailable) throw StateError('relay unavailable');
    return fixture.offerProfile ? null : UserProfile(pubkey: fixture.bound!);
  }

  @override
  Future<void> updateDisplayName(String name) async {
    fixture.profileWrites.add(name);
  }

  @override
  Future<void> updateSetup(String name, String? avatarUrl) async {
    fixture.profileWrites.add(name);
    fixture.profileAvatars.add(avatarUrl);
  }
}

// This boundary proves pending-flag/retry ownership, not relay memberships.
class _Recovery implements InviteJoinRecovery {
  _Recovery(this.fixture, this.scope);
  final HostedJourneyFixture fixture;
  final InviteJoinRecoveryScope scope;
  @override
  Future<String?> ensureStarterChannels() async {
    fixture.recoveryScopes.add(scope);
    if (fixture.starterFailures-- > 0) throw AccountFailure.unavailable;
    return 'general';
  }
}

class _Browser extends BrowserAuth {
  _Browser(Future<void> Function() onStart)
    : super(
        openBrowser: (uri) async {
          await onStart();
          return Uri.parse(BrowserAuth.callback)
              .replace(
                queryParameters: {
                  'state': uri.queryParameters['state']!,
                  'code': 'synthetic-code',
                },
              )
              .toString();
        },
      );
  @override
  Future<void> initialize() async {}
}

class _RelaySocket extends RelaySocket {
  _RelaySocket(
    this.fixture, {
    required super.wsUrl,
    required super.nsec,
    required super.onMessage,
    required super.onConnected,
    required super.onDisconnected,
  }) : url = wsUrl,
       _connected = onConnected,
       _disconnected = onDisconnected;
  final HostedJourneyFixture fixture;
  final String url;
  final void Function() _connected;
  final void Function(Object?) _disconnected;
  @override
  Future<void> connect() async {
    fixture.socketHeaders.add(Map.of(headers));
    expect(
      headers['Nostr-Federated-Identity'],
      HostedEnvironment.requiresAuth(Uri.parse(url).host)
          ? 'Bearer synthetic-assertion'
          : isNull,
    );
    if (fixture.refuseUpgrade) {
      _disconnected(const WebSocketException('refused', 403));
    } else {
      _connected();
    }
  }
}
