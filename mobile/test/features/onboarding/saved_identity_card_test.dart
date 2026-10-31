import 'dart:convert';
import 'dart:async';

import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/features/onboarding/saved_identity_card.dart';
import 'package:buzz/shared/hosted/self_hosted_entry.dart';
import 'package:buzz/shared/hosted/device_identity_source.dart';
import 'package:buzz/shared/hosted/saved_identity_profile.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/utils/string_utils.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('loads name and avatar when the active profile cache is empty', (
    tester,
  ) async {
    const pubkey =
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    final profile = Completer<UserProfile?>();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          savedIdentityProfileProvider(
            pubkey,
          ).overrideWith((ref) => profile.future),
        ],
        child: SavedIdentityCard(
          identity: const DeviceIdentityChoice(pubkey, 'Home'),
          onSelect: () {},
        ),
      ),
    );
    expect(find.text('Saved identity'), findsOneWidget);
    expect(find.text('Used in Home'), findsNothing);
    profile.complete(
      const UserProfile(
        pubkey: pubkey,
        displayName: 'Kenny',
        avatarUrl: 'https://home.example/avatar.png',
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Kenny'), findsOneWidget);
    expect(find.text('Saved identity'), findsNothing);
    expect(
      tester.widget<AvatarImage>(find.byType(AvatarImage)).imageUrl,
      'https://home.example/avatar.png',
    );
  });

  testWidgets('identity card below subtitle joins with the selected key', (
    tester,
  ) async {
    final keys = nostr.Keys('1'.padLeft(64, '0'));
    const source = 'mobile.communities.public.builderlabstaging.xyz';
    FlutterSecureStorage.setMockInitialValues({
      'buzz_communities': jsonEncode([
        {'name': source, 'pubkey': keys.public, 'nsec': keys.nsec},
      ]),
    });
    String? attemptedKey;
    final admission = Completer<void>();
    await tester.binding.setSurfaceSize(const Size(393, 852));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          selfHostedEntryProvider.overrideWithValue(
            SelfHostedEntry(
              probe: (community) async {
                attemptedKey = community.pubkey;
                await admission.future;
                throw StateError('Offline');
              },
            ),
          ),
        ],
        child: const SelfHostedEntrySheet(
          relay: 'wss://honeycombstudios.example',
          name: 'honeycombstudios.example',
          fullScreen: true,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Join honeycombstudios.example.'),
      findsOneWidget,
    );
    expect(find.text('You already have a saved identity'), findsOneWidget);
    expect(find.text('Used in mobile'), findsNothing);
    expect(find.text(shortPubkey(keys.public)), findsOneWidget);
    expect(find.text(fullNpub(keys.public)!), findsNothing);
    expect(
      find.bySemanticsLabel('Use Saved identity, ${shortPubkey(keys.public)}'),
      findsOneWidget,
    );

    expect(find.text('View identity details'), findsNothing);
    expect(
      tester.getBottomRight(find.text('Import a Nostr key')).dy,
      greaterThan(750),
    );
    expect(
      tester.getTopLeft(find.byType(SavedIdentityCard)).dy -
          tester
              .getBottomLeft(
                find.textContaining('Use this identity or import a Nostr key.'),
              )
              .dy,
      24,
    );

    final container = ProviderScope.containerOf(
      tester.element(find.byType(SelfHostedEntrySheet)),
    );
    container
        .read(userCacheProvider.notifier)
        .put(
          UserProfile(
            pubkey: keys.public,
            displayName: 'Alex',
            avatarUrl: 'https://example.com/avatar.png',
          ),
        );
    await tester.pumpAndSettle();
    expect(find.text('Alex'), findsOneWidget);
    expect(find.text('Used in mobile'), findsNothing);
    expect(
      tester.widget<AvatarImage>(find.byType(AvatarImage)).imageUrl,
      'https://example.com/avatar.png',
    );
    await tester.tap(find.text('Alex'));
    await tester.pump();
    expect(find.text('Getting ready'), findsOneWidget);
    expect(find.text('You already have a saved identity'), findsNothing);
    admission.complete();
    await tester.pumpAndSettle();
    expect(attemptedKey, keys.public);
    expect(
      find.textContaining('Your saved keys have been kept.'),
      findsOneWidget,
    );
    await tester.tap(find.text('Import a Nostr key'));
    await tester.pumpAndSettle();
    expect(find.text('Import identity'), findsOneWidget);
  });
}
