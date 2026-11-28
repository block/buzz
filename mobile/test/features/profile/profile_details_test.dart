import 'package:buzz/features/profile/profile_details.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('iOS globe tracks appearance and lets the profile scroll', (
    tester,
  ) async {
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (_) async => null,
    );
    addTearDown(
      () => messenger.setMockMethodCallHandler(
        SystemChannels.platform_views,
        null,
      ),
    );
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    for (final dark in [false, true]) {
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            themeAnimationDuration: Duration.zero,
            theme: dark ? AppTheme.dark() : AppTheme.light(),
            home: Scaffold(
              body: SingleChildScrollView(
                controller: scroll,
                child: Column(
                  children: [
                    const ProfileDetailsCards(
                      avatarUrl: null,
                      about: '',
                      username: null,
                      details: ProfileDetails(
                        city: 'London',
                        coordinates: (latitude: 51.51, longitude: -0.13),
                      ),
                    ),
                    const SizedBox(height: 800),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      final globe = find.byWidgetPredicate(
        (w) => w is UiKitView && w.viewType == 'buzz/profile_globe',
      );
      expect(tester.getSize(globe).aspectRatio, closeTo(160 / 60, 0.001));
      expect(tester.widget<UiKitView>(globe).creationParams, {
        'latitude': 51.51,
        'longitude': -0.13,
        'dark': dark,
      });
      await tester.dragFrom(
        tester.getTopLeft(globe) + const Offset(48, 65),
        const Offset(0, -80),
      );
      await tester.pumpAndSettle();
      expect(scroll.offset, greaterThan(0));
      expect(
        tester.getCenter(find.byType(PhysicalShape).last),
        tester.getCenter(globe),
      );
      scroll.jumpTo(0);
    }
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  for (final coordinates in [null, (latitude: 51.51, longitude: -0.13)]) {
    testWidgets('city and local time with coordinates $coordinates', (
      tester,
    ) async {
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: SingleChildScrollView(
                child: ProfileDetailsCards(
                  avatarUrl: null,
                  about: 'About me',
                  username: 'someone',
                  details: ProfileDetails(
                    tenure: (label: 'Time at Example', value: '2 years'),
                    city: 'London',
                    timeZone: 'Europe/London',
                    coordinates: coordinates,
                    lead: (name: 'Alex', pubkey: null, avatarUrl: null),
                    team: const [(label: 'Team', value: 'Design systems')],
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(find.text('London'), findsOneWidget);
      expect(find.textContaining(RegExp(r'(GMT|BST)$')), findsOneWidget);
      if (coordinates != null) {
        final map = find.bySemanticsLabel('Map highlighting London');
        expect(tester.getSize(map).aspectRatio, closeTo(160 / 60, 0.001));
      }
      final teamValue = tester.getTopLeft(find.text('Design systems'));
      final leadValue = tester.getTopLeft(find.text('Alex'));
      expect(teamValue.dx, Grid.xs + Grid.md + Grid.xs);
      expect(leadValue.dx, teamValue.dx);
      expect(
        tester.getTopLeft(find.text('Time at Example')).dy,
        greaterThan(leadValue.dy),
      );
      expect(find.text('2 years'), findsOneWidget);
      expect(
        find.bySemanticsLabel('Map highlighting London'),
        coordinates == null ? findsNothing : findsOneWidget,
      );
    });
  }
}
