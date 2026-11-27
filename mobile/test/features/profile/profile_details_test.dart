import 'package:buzz/features/profile/profile_details.dart';
import 'package:buzz/features/profile/profile_country_codes.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  test('abbreviates the country suffix of profile locations', () {
    expect(compactProfileCity('London, United Kingdom'), 'London, UK');
    expect(compactProfileCity('New York, United States'), 'New York, USA');
    expect(compactProfileCity('Toronto, Canada'), 'Toronto, CA');
    expect(compactProfileCity('Paris, France'), 'Paris, FR');
    expect(compactProfileCity('Remote'), 'Remote');
  });
  testWidgets('maps named remote areas and shows only the country otherwise', (
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
    for (final (city, label, latitude) in [
      ('San Francisco (Remote), US', 'San Francisco', 37.7749),
      ('Denver Metro (Remote), US', 'Denver Metro', 39.7392),
      ('NYC Metro (Remote), US', 'NYC Metro', 40.7128),
      ('Southern CA (Remote), US', 'Southern CA', 34.0),
      ('Remote, US', 'Eastern Time · US · Remote', null),
    ]) {
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: SingleChildScrollView(
                child: ProfileDetailsCards(
                  avatarUrl: null,
                  about: '',
                  username: null,
                  details: ProfileDetails(city: city, timeZone: 'US/Eastern'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(find.text(label), findsOneWidget);
      final globe = find.byWidgetPredicate(
        (w) => w is UiKitView && w.viewType == 'buzz/profile_globe',
      );
      if (latitude == null) {
        expect(globe, findsNothing);
        expect(find.text('Time zone'), findsOneWidget);
        expect(find.text('Local time'), findsOneWidget);
      } else {
        expect(
          (tester.widget<UiKitView>(globe).creationParams as Map)['latitude'],
          latitude,
        );
      }
    }
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
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
                      about: 'Profile bio',
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
      final labels = find.byWidgetPredicate(
        (w) => w is UiKitView && w.viewType == 'buzz/profile_vibrant_label',
      );
      expect(labels, findsWidgets);
      for (final view in [
        tester.widget<UiKitView>(globe),
        ...tester.widgetList<UiKitView>(labels),
      ]) {
        expect(
          view.gestureBlockingPolicy,
          UiKitViewGestureBlockingPolicy.doNotBlockGesture,
        );
      }
      expect(
        tester.getSize(globe).width,
        tester.getSize(find.byType(Scaffold)).width,
      );
      expect(tester.widget<UiKitView>(globe).creationParams, {
        'officePreview': true,
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
        tester.getCenter(
          find.descendant(
            of: find.bySemanticsLabel('Map highlighting London'),
            matching: find.byType(PhysicalShape),
          ),
        ),
        tester.getTopLeft(globe) +
            Offset(
              tester.getSize(globe).width * .68,
              tester.getSize(globe).height / 2,
            ),
      );
      scroll.jumpTo(0);
    }
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  for (final width in [264.0, 360.0, 420.0]) {
    testWidgets('iOS groups contact, about and team at width $width', (
      tester,
    ) async {
      ProfileLead? opened;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.dark().copyWith(platform: TargetPlatform.iOS),
            home: Scaffold(
              body: Align(
                alignment: Alignment.topLeft,
                child: SizedBox(
                  width: width,
                  child: SingleChildScrollView(
                    child: ProfileDetailsCards(
                      onLead: (lead) => opened = lead,
                      details: const ProfileDetails(
                        team: [
                          (label: 'Team', value: 'Design'),
                          (label: 'Role', value: 'Product designer'),
                          (label: 'Function', value: 'Design practice'),
                          (label: 'Org', value: 'Research and development'),
                        ],
                        lead: (name: 'Alex', pubkey: null, avatarUrl: null),
                        teammates: [
                          (name: 'Zoe', pubkey: null, avatarUrl: null),
                          (name: 'Bea', pubkey: 'peer-key', avatarUrl: null),
                        ],
                        personal: [
                          (label: 'Pronouns', value: 'they/them'),
                          (label: 'Pronunciation', value: 'AL-eks'),
                          (label: 'Email', value: 'alex@example.org'),
                        ],
                        talkToMeAbout: 'Maps and travel',
                        tenure: (label: 'Tenure', value: '2 years'),
                      ),
                      avatarUrl: null,
                      about: width == 264 ? '  ' : 'Profile biography',
                      username: 'alex',
                      ldap: 'alex',
                      publicKey: 'npub-example',
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      final team = tester.getTopLeft(find.text('Team'));
      final tenure = tester.getTopLeft(find.text('Tenure'));
      expect(find.text('Public key'), findsNothing);
      expect(find.text('npub-example'), findsNothing);
      expect(find.text('Team'), findsOneWidget);
      expect(find.text('Design'), findsOneWidget);
      expect(find.text('Org'), findsNothing);
      expect(find.text('@alex'), findsOneWidget);
      final address = find.text('alex@example.org');
      expect(tester.widget<Text>(address).maxLines, isNull);
      expect(
        tester.getTopLeft(find.byIcon(BuzzIcons.copy)).dx -
            tester.getTopRight(address).dx,
        Grid.half,
      );
      final ldap = tester.getTopLeft(find.text('LDAP'));
      final email = tester.getTopLeft(find.text('Email'));
      final pronouns = tester.getTopLeft(find.text('Pronouns'));
      expect(email.dy, greaterThan(tester.getTopLeft(find.text('Contact')).dy));
      expect(email.dx, ldap.dx);
      expect(ldap.dy, greaterThan(email.dy));
      expect(
        tester.getTopLeft(find.text('Contact')).dy,
        greaterThan(pronouns.dy),
      );
      expect(
        pronouns.dy,
        greaterThan(tester.getTopLeft(find.text('About')).dy),
      );
      expect(
        tester.getTopLeft(find.text('Pronunciation')).dy,
        greaterThan(pronouns.dy),
      );
      expect(tester.getSize(find.text('Email')).width, width);
      expect(tester.getSize(find.text('LDAP')).width, width);
      expect(find.text('AL-eks'), findsOneWidget);
      final card = find.bySemanticsLabel('Alex, Lead');
      expect(
        tester.getTopLeft(find.text('Alex')).dx,
        lessThan(tester.getTopLeft(find.text('Bea')).dx),
      );
      expect(
        tester.getTopLeft(find.text('Bea')).dx,
        lessThan(tester.getTopLeft(find.text('Zoe')).dx),
      );
      final role = tester.getTopLeft(find.text('Role'));
      expect(tester.getTopLeft(card).dy, greaterThan(tenure.dy));
      final group = tester.getTopLeft(find.text('Group'));
      final function = tester.getTopLeft(find.text('Function'));
      expect(group.dy, lessThan(role.dy));
      expect(function.dy, greaterThan(role.dy));
      expect(tenure.dy, greaterThan(function.dy));
      expect(ldap.dy, lessThan(team.dy));
      expect(
        tester.getBottomLeft(card).dy,
        lessThan(tester.getTopLeft(find.text('Office Location')).dy),
      );
      await tester.ensureVisible(card);
      await tester.tap(card);
      expect(opened?.name, 'Alex');
      await tester.tap(find.text('Bea'));
      expect(opened?.pubkey, 'peer-key');
      expect(find.byIcon(Icons.public), findsNothing);
      expect(tenure.dy, greaterThan(team.dy));

      expect(
        tester
            .getSize(
              find.text(width == 264 ? 'Maps and travel' : 'Profile biography'),
            )
            .width,
        width,
      );
      expect(find.text('Talk to me about'), findsNothing);
      if (width != 264) expect(find.text('Maps and travel'), findsNothing);
    });
  }
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
