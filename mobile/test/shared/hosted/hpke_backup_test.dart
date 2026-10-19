import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/hosted/hpke_backup.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final fixture =
      jsonDecode(
            File(
              '../desktop/src-tauri/src/testdata/hpke_nsec_backup_v1.json',
            ).readAsStringSync(),
          )
          as Map<String, dynamic>;
  final enrollment = Map<String, dynamic>.from(fixture['envelope'] as Map)
    ..remove('enc')
    ..remove('ciphertext');
  Uint8List hex(String value) => Uint8List.fromList(
    List.generate(
      value.length ~/ 2,
      (i) => int.parse(value.substring(i * 2, i * 2 + 2), radix: 16),
    ),
  );
  enrollment['recipient_public_key'] = base64UrlEncode(
    hex(fixture['recipient_public_key_sec1_hex'] as String),
  ).replaceAll('=', '');

  test('v1 AAD exactly matches desktop/server interop fixture', () {
    expect(
      HpkeBackup.associatedData(enrollment),
      hex(fixture['aad_hex'] as String),
    );
    for (final invalid in [
      {...enrollment, 'version': 2},
      {...enrollment, 'owner_id': 'owner\n'},
      {
        ...enrollment,
        'backup_id': (enrollment['backup_id'] as String).toUpperCase(),
      },
    ]) {
      expect(
        () => HpkeBackup.associatedData(invalid),
        throwsA(HpkeBackupFailure.invalidEnrollment),
      );
    }
  });

  test(
    'seal binds derived identity and passes exact primitives to native',
    () async {
      var calls = 0;
      const channel = MethodChannel('buzz/hosted_hpke');
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
            calls++;
            expect(call.method, 'seal');
            final args = call.arguments as Map;
            expect(args['aad'], hex(fixture['aad_hex'] as String));
            expect(
              args['plaintext'],
              hex(fixture['nostr_secret_key_hex'] as String),
            );
            expect(
              args['recipient'],
              hex(fixture['recipient_public_key_sec1_hex'] as String),
            );
            final envelope = fixture['envelope'] as Map;
            return {
              'enc': base64Url.decode(
                base64Url.normalize(envelope['enc'] as String),
              ),
              'ciphertext': base64Url.decode(
                base64Url.normalize(envelope['ciphertext'] as String),
              ),
            };
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(channel, null),
      );
      final result = await HpkeBackup().seal(
        enrollment: enrollment,
        secretHex: fixture['nostr_secret_key_hex'] as String,
      );
      expect(result, fixture['envelope']);
      await expectLater(
        HpkeBackup().seal(
          enrollment: {...enrollment, 'nostr_pubkey': '0' * 64},
          secretHex: fixture['nostr_secret_key_hex'] as String,
        ),
        throwsA(HpkeBackupFailure.identityMismatch),
      );
      expect(calls, 1);
    },
  );
}
