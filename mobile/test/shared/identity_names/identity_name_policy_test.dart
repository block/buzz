import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/identity_names/identity_name_policy.dart';
import 'package:flutter_test/flutter_test.dart';

// Vendored, unmodified copy of the portable fixtures pinned in
// identity_name_policy.dart. Expected results are stored literals.
const _fixturePath = 'test/shared/identity_names/identity-names.fixtures.json';

void main() {
  final conformance =
      jsonDecode(File(_fixturePath).readAsStringSync()) as Map<String, dynamic>;
  final cases = (conformance['cases'] as List).cast<Map<String, dynamic>>();

  test('validates the portable fixture version and non-empty case list', () {
    expect(conformance['version'], identityNamePolicyVersion);
    expect(cases, isNotEmpty);
  });

  for (final fixture in cases) {
    test('conforms to the portable naming contract: ${fixture['name']}', () {
      final identities = [
        for (final raw
            in (fixture['identities'] as List).cast<Map<String, dynamic>>())
          NamingIdentity(
            pubkey: raw['pubkey'] as String,
            name: raw['name'] as String,
            isAgent: raw['isAgent'] as bool? ?? false,
            ownerPubkey: raw['ownerPubkey'] as String?,
          ),
      ];
      final actual = resolveIdentityNames(
        identities,
        viewer: fixture['viewer'] as String?,
        candidates: (fixture['candidates'] as List?)?.cast<String>(),
      );
      expect({
        for (final MapEntry(:key, :value) in actual.entries)
          key: {'name': value.name, 'qualifier': value.qualifier},
      }, fixture['expected']);
    });
  }

  test('trims only the contract whitespace set', () {
    const key =
        '1111111111111111111111111111111111111111111111111111111111111111';
    // U+0085 is not ECMAScript whitespace, unlike Dart String.trim.
    final result = resolveIdentityNames([
      const NamingIdentity(pubkey: key, name: '\u0085Honey\u3000'),
    ]);
    expect(result[key], const ResolvedIdentityName('\u0085Honey'));
  });

  test('rejects an invalid public key instead of inventing an identity', () {
    expect(
      () => resolveIdentityNames(const [
        NamingIdentity(pubkey: 'not-a-key', name: 'Honey'),
        NamingIdentity(
          pubkey:
              '1111111111111111111111111111111111111111111111111111111111111111',
          name: 'Honey',
        ),
      ]),
      throwsArgumentError,
    );
  });
}
