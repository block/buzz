import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('UserProfile label and initial', () {
    // Valid fixture key whose npub encoding was verified against the NIP-19
    // codec independently of the code under test.
    const b0b =
        'b0b0000000000000000000000000000000000000000000000000000000000000';

    // Relay profiles can carry a blank `display_name` (empty or
    // whitespace-only) and reach the cache unchanged, so both accessors must
    // reject blank names and fall back to the key instead of rendering an
    // empty label. Nonblank names render as authored: `label` only tests
    // blankness with trim, while `initial` reads the trimmed padding.
    final cases = <({String? displayName, String label, String initial})>[
      (displayName: null, label: shortPubkey(b0b), initial: 'B'),
      (displayName: '', label: shortPubkey(b0b), initial: 'B'),
      (displayName: '   ', label: shortPubkey(b0b), initial: 'B'),
      (displayName: 'Carol', label: 'Carol', initial: 'C'),
      (displayName: ' Carol ', label: ' Carol ', initial: 'C'),
    ];

    for (final testCase in cases) {
      test('displayName ${testCase.displayName ?? '(null)'}', () {
        final profile = UserProfile(
          pubkey: b0b,
          displayName: testCase.displayName,
        );
        expect(profile.label, testCase.label);
        expect(profile.initial, testCase.initial);
      });
    }
  });
}
