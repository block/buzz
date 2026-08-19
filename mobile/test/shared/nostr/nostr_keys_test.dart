import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/nostr/nostr_keys.dart';
import 'package:buzz/shared/utils/string_utils.dart';

void main() {
  final keys = nostr.Keys(
    '1111111111111111111111111111111111111111111111111111111111111111',
  );

  group('Nostr identity key forms', () {
    test('strict nsec parsing returns canonical bech32 and wire hex', () {
      final identity = identityFromNsec(keys.nsec);

      expect(identity.nsec, keys.nsec);
      expect(identity.npub, keys.npub);
      expect(identity.privateKeyHex, keys.secret);
      expect(identity.publicKeyHex, keys.public);
      expect(identityFromNsec(keys.nsec.toUpperCase()).nsec, keys.nsec);
    });

    test('an npub is never accepted as a secret key', () {
      expect(() => identityFromNsec(keys.npub), throwsFormatException);
    });

    test('legacy stored private hex is canonicalized to nsec', () {
      final identity = identityFromStoredSecret(keys.secret.toUpperCase());

      expect(identity.nsec, keys.nsec);
      expect(identity.npub, keys.npub);
    });

    test('public hex and npub normalize to the same identity', () {
      expect(npubFromPublicKey(keys.public), keys.npub);
      expect(npubFromPublicKey(keys.npub), keys.npub);
      expect(publicKeyHexFromInput(keys.npub), keys.public);
      expect(
        publicKeyHexFromInput(keys.public, allowLegacyHex: true),
        keys.public,
      );
      expect(publicKeyHexFromInput(keys.npub.toUpperCase()), keys.public);
    });

    test('mixed-case and invalid-curve public keys are rejected', () {
      expect(
        () => publicKeyHexFromInput('N${keys.npub.substring(1)}'),
        throwsFormatException,
      );
      final invalidPoint = 'ff' * 32;
      final invalidNpub = nostr.Bech32Entity.encode(
        prefix: nostr.Nip19Prefix.npub,
        data: invalidPoint,
      );
      expect(
        () => publicKeyHexFromInput(invalidPoint, allowLegacyHex: true),
        throwsFormatException,
      );
      expect(() => publicKeyHexFromInput(invalidNpub), throwsFormatException);
    });

    test('raw public hex requires an explicit compatibility boundary', () {
      expect(() => publicKeyHexFromInput(keys.public), throwsFormatException);
    });
  });

  group('identity presentation', () {
    test('valid raw public keys render only as compact npub', () {
      final label = shortPubkey(keys.public);

      expect(label, startsWith('npub1'));
      expect(label, contains('…'));
      expect(label, endsWith(keys.npub.substring(keys.npub.length - 8)));
      expect(label, isNot(contains(keys.public.substring(0, 8))));
    });

    test('fallback avatar glyphs derive from npub payload', () {
      expect(pubkeyAvatarInitial(keys.public), keys.npub[5].toUpperCase());
      expect(pubkeyAvatarInitial(keys.npub), keys.npub[5].toUpperCase());
    });

    test('event IDs keep event-ID formatting', () {
      final eventId = 'ab' * 32;

      expect(shortEventId(eventId), 'abababab…');
      expect(shortEventId(eventId), isNot(startsWith('npub1')));
    });
  });
}
