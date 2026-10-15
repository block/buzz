import 'package:buzz/features/invites/invite_create_provider.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

/// Tests for the npub identity helpers shared by every mobile surface.
///
/// The canonical vector comes from the NIP-19 specification itself, so these
/// tests fail if the encoding (or the truncation policy) drifts — they never
/// re-derive expectations from the codec under test.
void main() {
  const canonicalHex =
      '3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d';
  const canonicalNpub =
      'npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6';
  const canonicalCompact = 'npub180c\u2026h6w6';

  group('fullNpub', () {
    test('canonicalizes every accepted input wrapper', () {
      // Every accepted wrapper — lowercase hex, uppercase hex, whitespace-
      // padded, and an already-npub key — converges on the canonical npub.
      final inputs = <String>[
        canonicalHex,
        canonicalHex.toUpperCase(),
        '  $canonicalHex \n',
        canonicalNpub,
      ];
      for (final input in inputs) {
        expect(fullNpub(input), canonicalNpub, reason: input);
      }
    });

    test('rejects malformed identities without echoing them back', () {
      expect(fullNpub(''), isNull);
      expect(fullNpub('unknown'), isNull);
      // Wrong-length hex.
      expect(fullNpub('a' * 63), isNull);
      expect(fullNpub('a' * 65), isNull);
      // Hex-length but not hex.
      expect(fullNpub('z' * 64), isNull);
      // Not decodable bech32.
      expect(fullNpub('npub1garbage'), isNull);
      // Mixed-case bech32 is invalid by checksum.
      final mixedCase =
          '${canonicalNpub.substring(0, 20)}'
          '${canonicalNpub.substring(20).toUpperCase()}';
      expect(fullNpub(mixedCase), isNull);
      // Valid NIP-19 forms that are not public keys stay out of the npub
      // contract instead of leaking their payload.
      final nsec = nostr.Nip19.encode(
        prefix: nostr.Nip19Prefix.nsec,
        data: canonicalHex,
      );
      expect(fullNpub(nsec), isNull);
      final note = nostr.Nip19.encode(
        prefix: nostr.Nip19Prefix.note,
        data: canonicalHex,
      );
      expect(fullNpub(note), isNull);
    });
  });

  group('shortPubkey', () {
    test('renders the compact npub label — first 8 … last 4', () {
      expect(shortPubkey(canonicalHex), canonicalCompact);
    });

    test('is idempotent for already-npub input', () {
      expect(shortPubkey(canonicalNpub), canonicalCompact);
    });

    test('returns the neutral label for invalid identities', () {
      expect(shortPubkey(''), unknownIdentityLabel);
      expect(shortPubkey('not-a-key'), unknownIdentityLabel);
      expect(shortPubkey('npub1garbage'), unknownIdentityLabel);
      expect(shortPubkey('n' * 64), unknownIdentityLabel);
    });
  });

  group('copy to paste roundtrip', () {
    test('a copied npub parses back to the original hex key', () {
      final copied = fullNpub(canonicalHex);
      // The invite input accepts what the profile/settings copy actions put
      // on the clipboard — the mobile copy/paste contract.
      expect(parseCommunityInvitePubkey(copied!), canonicalHex);
      // Hex input keeps working unchanged.
      expect(parseCommunityInvitePubkey(canonicalHex), canonicalHex);
    });
  });
}
