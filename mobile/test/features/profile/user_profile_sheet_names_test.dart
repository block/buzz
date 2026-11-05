import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_directory.dart';
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

class _VerifiedProfiles extends VerifiedProfilesNotifier {
  @override
  Map<String, VerifiedProfile> build() => {};

  void replace(Map<String, VerifiedProfile> profiles) => state = profiles;
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
    final verified = _VerifiedProfiles();
    late IdentityNames opened;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(() => cache),
          verifiedProfilesProvider.overrideWith(() => verified),
        ],
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
                  names: Provider.autoDispose<IdentityNames>(
                    (ref) => opened.withSources(
                      ref.watch(identityNameSourcesProvider),
                    ),
                  ),
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

    // A community full name becomes the title, with the display name below.
    for (final checked in [true, false]) {
      verified.replace({
        scout: VerifiedProfile(
          displayName: 'Scout',
          username: 'scout',
          fullName: 'Scout Example',
          issuerLabel: 'Block',
          verifiedAt: 1000000,
          checked: checked,
          multipleIssuers: false,
        ),
      });
      await tester.pump();
      final title = find.text('Scout Example');
      final subtitle = find.text('Scout');
      expect(title, findsOneWidget);
      expect(subtitle, findsOneWidget);
      expect(
        tester.getTopLeft(title).dy,
        lessThan(tester.getTopLeft(subtitle).dy),
      );
      expect(
        tester.getCenter(title).dx,
        closeTo(tester.getCenter(subtitle).dx, 1),
      );
      expect(
        tester.widget<Text>(title).style!.fontSize,
        greaterThan(tester.widget<Text>(subtitle).style!.fontSize!),
      );
      expect(find.byKey(const ValueKey('verified-mark')), findsNothing);
      expect(find.textContaining('Name verified'), findsNothing);
      expect(find.textContaining('Last verified'), findsNothing);
      expect(find.textContaining('current account access'), findsNothing);
    }
    verified.replace({});
    await tester.pump();
    expect(find.text('Scout Example'), findsNothing);
    expect(find.text('Scout'), findsOneWidget);
  });
}
