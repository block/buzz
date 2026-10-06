import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/shared/identity_names/identity_names_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _UserCache extends UserCacheNotifier {
  _UserCache(this._initial);

  final Map<String, UserProfile> _initial;

  @override
  Map<String, UserProfile> build() => _initial;

  @override
  UserProfile? get(String pubkey) => state[pubkey.toLowerCase()];

  @override
  Future<bool> preload(List<String> pubkeys) async => true;

  void replace(UserProfile profile) =>
      state = {...state, profile.pubkey: profile};
}

void main() {
  final scout = 'a' * 64;
  final other = 'b' * 64;

  testWidgets('a supplied context follows profile changes while open', (
    tester,
  ) async {
    final cache = _UserCache({
      scout: UserProfile(pubkey: scout, displayName: 'Scout'),
      other: UserProfile(pubkey: other, displayName: 'Scout'),
    });
    late IdentityNames opened;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [userCacheProvider.overrideWith(() => cache)],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Consumer(
            builder: (context, ref, _) {
              // The opener's context: both Scouts, so each has a suffix.
              opened = ref.read(identityNameSourcesProvider).scope([
                scout,
                other,
              ]);
              return Scaffold(
                body: UserProfileSheet(
                  pubkey: scout,
                  names: liveIdentityNamesProvider(opened),
                ),
              );
            },
          ),
        ),
      ),
    );
    await tester.pump();
    final suffixed = opened.labelFor(scout);
    expect(suffixed, startsWith('Scout · '));
    expect(find.text(suffixed), findsOneWidget);

    // The other Scout renames. The open sheet resolves the opener's context
    // against live facts, so its label no longer needs the suffix.
    cache.replace(UserProfile(pubkey: other, displayName: 'Renamed'));
    await tester.pump();
    expect(find.text('Scout'), findsOneWidget);
    expect(find.text(suffixed), findsNothing);
  });
}
