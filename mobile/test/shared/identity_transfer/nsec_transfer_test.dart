import 'dart:io';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/identity_transfer/nsec_transfer.dart';
import 'package:buzz/shared/security/sensitive_action_authorizer.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final keys = nostr.Keys('1'.padLeft(64, '0'));
  final other = nostr.Keys('2'.padLeft(64, '0'));

  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    test(
      'native $platform import validates text and preserves cancellation/error',
      () async {
        debugDefaultTargetPlatformOverride = platform;
        const channel = MethodChannel('buzz/identity_file');
        final messenger =
            TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
        addTearDown(() {
          debugDefaultTargetPlatformOverride = null;
          messenger.setMockMethodCallHandler(channel, null);
        });
        String? response = '${keys.nsec}\n';
        var failRead = false;
        messenger.setMockMethodCallHandler(channel, (call) async {
          expect(call.method, 'import');
          expect(call.arguments, isNull);
          if (failRead) throw PlatformException(code: 'import_failed');
          return response;
        });
        expect(await NsecTransfer.pickFile(), keys.nsec);
        response = null;
        expect(await NsecTransfer.pickFile(), isNull);
        response = keys.public;
        await expectLater(
          NsecTransfer.pickFile(),
          throwsA(AccountIdentityFailure.invalidKey),
        );
        response = 'x' * 1025;
        await expectLater(
          NsecTransfer.pickFile(),
          throwsA(AccountIdentityFailure.invalidKey),
        );
        failRead = true;
        await expectLater(
          NsecTransfer.pickFile(),
          throwsA(isA<PlatformException>()),
        );
      },
    );
  }

  test('paste parsing rejects wrong key type, invalid scalar and checksum', () {
    expect(NsecTransfer.preview('  ${keys.nsec}\n'), keys.public);
    for (final value in [
      keys.public,
      'nsec1bad',
      nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: '0' * 64),
    ]) {
      expect(
        () => NsecTransfer.preview(value),
        throwsA(AccountIdentityFailure.invalidKey),
      );
    }
  });

  test(
    'export respects biometric policy, auth denial and selected identity',
    () async {
      final community = Community.create(
        name: 'Test',
        relayUrl: 'wss://test.invalid',
        nsec: keys.nsec,
        pubkey: keys.public,
        sensitiveActionPolicy: SensitiveActionPolicy.enabled,
      );
      var shares = 0;
      var current = true;
      var result = DeviceAuthResult.cancelled;
      Future<bool> export() => NsecTransfer.exportCommunity(
        community: community,
        authorize: (biometricOnly) async {
          expect(biometricOnly, isTrue);
          return result;
        },
        isCurrent: () => current,
        share: (nsec) async {
          expect(nsec, keys.nsec);
          shares++;
          return true;
        },
      );
      expect(await export(), isFalse);
      result = DeviceAuthResult.success;
      current = false;
      expect(await export(), isFalse);
      expect(shares, 0);
      current = true;
      expect(await export(), isTrue);
      expect(shares, 1);
      await expectLater(
        NsecTransfer.exportCommunity(
          community: community.copyWith(nsec: other.nsec),
          authorize: (_) async =>
              fail('must not authorize a mismatched identity'),
          isCurrent: () => true,
          share: (_) async => fail('must not share a mismatched identity'),
        ),
        throwsA(AccountIdentityFailure.mismatch),
      );

      // Continue through the real share coordinator; only native boundaries
      // are synthetic here. Native tests assert the file's protection class.
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      const channel = MethodChannel('buzz/identity_file');
      const shareChannel = MethodChannel('dev.fluttercommunity.plus/share');
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      final root = await Directory.systemTemp.createTemp('nsec-export-test-');
      addTearDown(() async {
        debugDefaultTargetPlatformOverride = null;
        messenger.setMockMethodCallHandler(channel, null);
        messenger.setMockMethodCallHandler(shareChannel, null);
        await root.delete(recursive: true);
      });
      File? prepared;
      var failPrepare = false;
      var failShare = false;
      messenger.setMockMethodCallHandler(channel, (call) async {
        expect(call.method, 'prepareExport');
        expect(call.arguments, {'text': '${keys.nsec}\n'});
        if (failPrepare) throw PlatformException(code: 'export_failed');
        final directory = await root.createTemp('buzz-identity-');
        prepared = File('${directory.path}/buzz-identity.nsec');
        await prepared!.writeAsString('${keys.nsec}\n');
        return prepared!.path;
      });
      messenger.setMockMethodCallHandler(shareChannel, (call) async {
        expect(call.method, 'share');
        expect(call.arguments['paths'], [prepared!.path]);
        expect(await prepared!.readAsString(), '${keys.nsec}\n');
        if (failShare) throw PlatformException(code: 'share_failed');
        return 'com.apple.UIKit.activity.SaveToFiles';
      });
      Future<bool> shareFile() =>
          NsecTransfer.shareFile(keys.nsec, const Rect.fromLTWH(0, 0, 10, 10));
      expect(await shareFile(), isTrue);
      expect(await prepared!.parent.exists(), isFalse);
      failShare = true;
      await expectLater(shareFile(), throwsA(isA<PlatformException>()));
      expect(await prepared!.parent.exists(), isFalse);
      failPrepare = true;
      prepared = null; // Any accidental share after prepare fails is an error.
      await expectLater(shareFile(), throwsA(isA<PlatformException>()));
    },
  );
}
