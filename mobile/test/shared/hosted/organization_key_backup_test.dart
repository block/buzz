import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_backup_service.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/hpke_backup.dart';
import 'package:buzz/shared/hosted/organization_environment.dart';
import 'package:buzz/shared/hosted/organization_key_backup.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter/services.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

class _FakeHpke extends HpkeBackup {
  _FakeHpke(this.envelope);
  final Map<String, dynamic> envelope;
  int seals = 0;
  @override
  Future<Map<String, dynamic>> seal({
    required Map<String, dynamic> enrollment,
    required String secretHex,
  }) async {
    seals++;
    return envelope;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final fixture =
      jsonDecode(
            File(
              '../desktop/src-tauri/src/testdata/hpke_nsec_backup_v1.json',
            ).readAsStringSync(),
          )
          as Map<String, dynamic>;
  final envelope = Map<String, dynamic>.from(fixture['envelope'] as Map);
  final keys = nostr.Keys(fixture['nostr_secret_key_hex'] as String);
  final organization = Uri.parse('https://block.example.com');
  const relayHost = 'buzz.block.example.com';
  final relay = RelayConfig(baseUrl: 'https://$relayHost', nsec: keys.nsec);
  final expiry = DateTime.now().toUtc().add(const Duration(days: 1));
  final pending = HostedBackupService.pendingKey(
    organization.origin,
    'member',
    keys.public,
  );

  late List<String> paths;
  late List<String> uploads;
  late int statusCode;
  late List<String> listed;
  late String current;
  late int uploadCode;
  late DateTime now;
  late _FakeHpke hpke;

  BuilderlabAccountClient organizationClient() => BuilderlabAccountClient(
    accountOrigin: organization,
    httpClient: MockClient((request) async {
      expect(request.url.origin, organization.origin);
      expect(request.headers['X-BB-Session-Credential'], 'org-secret');
      final path = request.url.path.replaceFirst('/api/goose', '');
      paths.add(path);
      return switch (path) {
        '/v1/auth/me' => http.Response(
          jsonEncode({
            'subject': 'member',
            'expires_at': expiry.toIso8601String(),
          }),
          200,
        ),
        '/v1/buzz/nostr-identities/current' => http.Response(
          jsonEncode({'identity': null}),
          200,
        ),
        '/v1/buzz/identity/assertions' => http.Response(
          jsonEncode({
            'assertion': 'org-token',
            'nostr_pubkey': keys.public,
            'expires_at': DateTime.now().millisecondsSinceEpoch ~/ 1000 + 300,
          }),
          200,
        ),
        '/v1/buzz/nsec-backups/status' => http.Response(
          statusCode == 200
              ? jsonEncode({
                  'current_nostr_pubkey': current,
                  'backup_nostr_pubkeys': listed,
                })
              : 'Not Found',
          statusCode,
        ),
        '/v1/buzz/nsec-backups/enrollment' => http.Response(
          jsonEncode(envelope),
          200,
        ),
        '/v1/buzz/nsec-backups' => () {
          uploads.add(request.body);
          return http.Response(
            uploadCode == 200
                ? jsonEncode({
                    'stored': true,
                    'backup_id': envelope['backup_id'],
                  })
                : 'Unavailable',
            uploadCode,
          );
        }(),
        _ => http.Response('{}', 404),
      };
    }),
  );

  /// Issues an organization assertion on a fresh relay connection, as a
  /// reconnect does, without waiting for the backup it starts.
  Future<void> assert_(OrganizationKeyBackup backup) async {
    final auth = HostedRelayAuth(
      BuilderlabAccountClient(
        httpClient: MockClient(
          (request) async => fail('Hosted account must not be used'),
        ),
      ),
      organization: backup.account,
      organizationBackup: backup,
      organizationRelay: (relay) =>
          OrganizationEnvironment.matches(relay, [relayHost]),
      enabled: true,
    );
    expect(await auth.headers(relay), {
      'Nostr-Federated-Identity': 'Bearer org-token',
    });
  }

  /// One assertion, then the backup it started runs to completion.
  Future<void> connect(OrganizationKeyBackup backup) async {
    await assert_(backup);
    await pumpEventQueue(times: 200);
  }

  OrganizationKeyBackup backupFor(BuilderlabAccountClient account) =>
      OrganizationKeyBackup(
        account,
        hpke: hpke,
        supportsSealing: () async => true,
        now: () => now,
      );

  setUp(() async {
    FlutterSecureStorage.setMockInitialValues({});
    await const FlutterSecureStorage().write(
      key: 'buzz_hosted_session_block_example_com_v1',
      value: jsonEncode({
        'origin': organization.toString(),
        'credential': 'org-secret',
        'subject': 'member',
        'expires_at': expiry.toIso8601String(),
      }),
    );
    now = DateTime.utc(2026, 11, 3);
    paths = [];
    uploads = [];
    statusCode = 200;
    listed = [];
    current = keys.public;
    uploadCode = 200;
    hpke = _FakeHpke(envelope);
  });

  List<String> backupPaths() =>
      paths.where((path) => path.contains('nsec-backups')).toList();

  test(
    'a bound key is backed up once, replaying an uncertain upload exactly',
    () async {
      final backup = backupFor(organizationClient());

      // The tenant is reachable but the upload's outcome is unknown. Two
      // connections asserting at once share one attempt.
      uploadCode = 503;
      await Future.wait([assert_(backup), assert_(backup)]);
      await pumpEventQueue(times: 200);
      expect(backupPaths(), [
        '/v1/buzz/nsec-backups/status',
        '/v1/buzz/nsec-backups/enrollment',
        '/v1/buzz/nsec-backups',
      ]);
      expect(backup.lastFailure, AccountFailure.unavailable);
      expect(await const FlutterSecureStorage().read(key: pending), isNotNull);

      // The next assertion after the retry delay replays the saved envelope,
      // not a new one.
      uploadCode = 200;
      paths.clear();
      now = now.add(OrganizationKeyBackup.firstRetryDelay);
      await connect(backup);
      expect(backupPaths(), [
        '/v1/buzz/nsec-backups/status',
        '/v1/buzz/nsec-backups',
      ]);
      expect(hpke.seals, 1);
      expect(uploads, hasLength(2));
      expect(uploads.last, uploads.first);
      expect(jsonDecode(uploads.last), envelope);
      expect(backup.lastFailure, isNull);
      expect(await const FlutterSecureStorage().read(key: pending), isNull);

      // Later assertions in this launch send no backup requests.
      paths.clear();
      await connect(backup);
      expect(backupPaths(), isEmpty);
    },
  );

  test('a listed key needs no upload and drops a leftover envelope', () async {
    await const FlutterSecureStorage().write(key: pending, value: '{}');
    listed = [keys.public];
    final backup = backupFor(organizationClient());
    await connect(backup);
    expect(backupPaths(), ['/v1/buzz/nsec-backups/status']);
    expect(hpke.seals, 0);
    expect(await const FlutterSecureStorage().read(key: pending), isNull);
  });

  test('a tenant without backups is asked once per launch', () async {
    statusCode = 404;
    final backup = backupFor(organizationClient());
    await connect(backup);
    await connect(backup);
    expect(backupPaths(), ['/v1/buzz/nsec-backups/status']);
    expect(backup.lastFailure, AccountFailure.rejected);
  });

  test('a device that cannot seal sends no backup requests', () async {
    final backup = OrganizationKeyBackup(
      organizationClient(),
      hpke: hpke,
      supportsSealing: () async => false,
    );
    await connect(backup);
    await connect(backup);
    expect(backupPaths(), isEmpty);
  });

  test('failures retry after a growing delay and never stop', () async {
    final backup = backupFor(organizationClient());

    statusCode = 503;
    await connect(backup);
    expect(backup.lastFailure, AccountFailure.unavailable);
    statusCode = 200;
    // A relay refusal re-issues at once; that assertion waits.
    await connect(backup);
    expect(backupPaths(), ['/v1/buzz/nsec-backups/status']);
    now = now.add(OrganizationKeyBackup.firstRetryDelay);
    await connect(backup);
    expect(uploads, hasLength(1));
    expect(backup.lastFailure, isNull);

    // A key bound to someone else's account is never uploaded. Its retries
    // slow down to the longest delay but continue in a long-running app.
    final other = backupFor(organizationClient());
    current = 'f' * 64;
    paths.clear();
    // Each retry waits exactly its delay: none a second early, one on time.
    await connect(other);
    var delay = OrganizationKeyBackup.firstRetryDelay;
    for (var i = 0; i < 9; i++) {
      now = now.add(delay - const Duration(seconds: 1));
      await connect(other);
      expect(backupPaths(), hasLength(i + 1));
      now = now.add(const Duration(seconds: 1));
      await connect(other);
      expect(backupPaths(), hasLength(i + 2));
      delay = delay * 2 > OrganizationKeyBackup.maxRetryDelay
          ? OrganizationKeyBackup.maxRetryDelay
          : delay * 2;
    }
    expect(other.lastFailure, HostedBackupFailure.identityMismatch);
    expect(backupPaths(), List.filled(10, '/v1/buzz/nsec-backups/status'));
    expect(uploads, hasLength(1));
  });

  test('a native sealing check that fails sends nothing', () async {
    final backup = OrganizationKeyBackup(
      organizationClient(),
      hpke: hpke,
      supportsSealing: () async => throw PlatformException(code: 'failed'),
    );
    await connect(backup);
    expect(backup.lastFailure, HpkeBackupFailure.unavailable);
    expect(backupPaths(), isEmpty);
  });
}
