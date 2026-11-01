import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final ada = 'a' * 64;
  final other = 'b' * 64;

  test('a verified name outranks kind 0, directory and fallback names', () {
    final sources = IdentityNameSources(
      profiles: {ada: UserProfile(pubkey: ada, displayName: 'Self Declared')},
      agentDisplayNames: {ada: 'Directory'},
      verifiedNames: {ada: 'Ada Lovelace'},
    );
    final names = sources.scope([ada], fallbackNames: {ada: 'Fallback'});
    expect(names.labelFor(ada), 'Ada Lovelace');
    expect(sources.factFor(ada)?.name, 'Ada Lovelace');

    // Without a verified record the kind 0 name stands, never a mark-derived one.
    final unverified = IdentityNameSources(
      profiles: {ada: UserProfile(pubkey: ada, displayName: 'Self Declared')},
    );
    expect(unverified.scope([ada]).labelFor(ada), 'Self Declared');
  });

  test('a self-declared copy of a verified name is told apart', () {
    final sources = IdentityNameSources(
      profiles: {
        ada: UserProfile(pubkey: ada, displayName: 'Self Declared'),
        other: UserProfile(pubkey: other, displayName: 'Ada Lovelace'),
      },
      verifiedNames: {ada: 'Ada Lovelace'},
    );
    final names = sources.scope([ada, other]);
    expect(names.labelFor(ada), isNot(names.labelFor(other)));
  });
}
