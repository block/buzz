import 'dart:ffi';
import 'dart:io';

import 'package:buzz/shared/crypto/event_signer.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('production event constructors use the shared signer', () {
    final bypasses = Directory('lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where(
          (file) =>
              file.path.endsWith('.dart') &&
              file.path != 'lib/shared/crypto/event_signer.dart',
        )
        .where(
          (file) => RegExp(
            r'\bEvent\s*\.\s*from\s*\(',
          ).hasMatch(file.readAsStringSync()),
        )
        .map((file) => file.path)
        .toList();
    expect(bypasses, isEmpty);
  });
  final library = Platform.environment['BUZZ_SIGNER_LIBRARY'];
  final native = library == null
      ? null
      : NativeEventSigner(DynamicLibrary.open(library));
  final key = '0' * 63 + '3';

  test('platform signer preserves canonical payload and valid signature', () {
    final event = signEvent(
      kind: 9,
      content: 'hi\n🐝',
      secretKey: key,
      createdAt: 1700000000,
      tags: [
        ['h', 'community'],
        ['x', '"\\'],
      ],
    );
    expect(event.isValid(), isTrue);
    expect(event.content, 'hi\n🐝');
  });

  group(
    'native signing',
    skip: library == null
        ? 'Set BUZZ_SIGNER_LIBRARY to the compiled C adapter'
        : false,
    () {
      test('BIP340 fixture keys produce cross-verified event signatures', () {
        final lines = File(
          'ios/BuzzPushKit/Tests/BuzzPushKitTests/CryptoFixtures/bip340.csv',
        ).readAsLinesSync().skip(1);
        for (final line in lines) {
          final fields = line.split(',');
          if (fields[1].isEmpty) continue;
          final event = native!.sign(
            kind: 24242,
            content: 'Fixture ${fields[0]} 🐝',
            secretKey: fields[1],
            createdAt: 1700000000,
            tags: [
              ['t', 'get'],
              ['expiration', '1700000060'],
            ],
          );
          expect(event.pubkey, fields[2].toLowerCase());
          expect(event.isValid(), isTrue);
          final dart = nostr.Event.from(
            kind: event.kind,
            content: event.content,
            secretKey: fields[1],
            createdAt: event.createdAt,
            tags: event.tags,
          );
          expect(event.id, dart.id);
        }
      });
      test('alternating identities and scopes never reuse key material', () {
        for (var i = 1; i <= 12; i++) {
          final secret = i.toRadixString(16).padLeft(64, '0');
          final event = native!.sign(
            kind: 9,
            content: '$i',
            secretKey: secret,
            createdAt: 1700000000 + i,
            tags: [
              ['h', 'scope-$i'],
            ],
          );
          expect(event.pubkey, nostr.Keys(secret).public);
          expect(event.isValid(), isTrue);
        }
      });
      test('invalid key fails and does not poison subsequent signing', () {
        for (final secret in ['', 'z' * 64, '0' * 64, 'f' * 64]) {
          expect(
            () => native!.sign(kind: 9, content: '', secretKey: secret),
            throwsFormatException,
          );
        }
        expect(
          native!.sign(kind: 9, content: '', secretKey: key).isValid(),
          isTrue,
        );
      });
      test(
        'fresh auxiliary randomness produces different valid signatures',
        () {
          final first = native!.sign(
            kind: 9,
            content: '',
            secretKey: key,
            createdAt: 1700000000,
          );
          final second = native.sign(
            kind: 9,
            content: '',
            secretKey: key,
            createdAt: first.createdAt,
          );
          expect(first.id, second.id);
          expect(first.sig, isNot(second.sig));
          expect(second.isValid(), isTrue);
        },
      );
    },
  );
}
