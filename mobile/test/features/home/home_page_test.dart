import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/features/home/home_page.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/profile/profile_avatar.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  Future<Widget> buildHome({
    int unreadInboxCount = 0,
    bool disableAnimations = false,
    Gradient? topSectionGradient,
  }) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    return ProviderScope(
      overrides: [savedPrefsProvider.overrideWithValue(prefs)],
      child: MaterialApp(
        theme: AppTheme.light(topSectionGradient: topSectionGradient),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(
            context,
          ).copyWith(disableAnimations: disableAnimations),
          child: child!,
        ),
        home: HomePage(
          mePageBuilder: (_) => const Center(child: Text('Personal workspace')),
          settingsPageBuilder: _buildSettingsPage,
          hasUnreadInbox: unreadInboxCount > 0,
        ),
      ),
    );
  }

  testWidgets('iOS glass retains icons, tab taps and unread semantics', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    const support = MethodChannel('buzz/concentric_sheet_surface');
    final created = <Map<Object?, Object?>>[];
    messenger.setMockMethodCallHandler(support, (call) async => true);
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'create') {
        final args = call.arguments as Map<Object?, Object?>;
        if (args['viewType'] == 'buzz/concentric_sheet_surface') {
          created.add(
            const StandardMessageCodec().decodeMessage(
                  ByteData.sublistView(args['params'] as Uint8List),
                )
                as Map<Object?, Object?>,
          );
        }
      }
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(support, null);
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
    });
    await tester.pumpWidget(await buildHome(unreadInboxCount: 2));
    await tester.pump();
    await tester.pump();
    final bar = find.byKey(const ValueKey('home-ios-glass-tabs'));
    expect(bar, findsOneWidget);
    expect(tester.getSize(bar), const Size(218, 56));
    expect(created.where((params) => params['usesGlass'] == true), isNotEmpty);
    expect(
      find.descendant(of: bar, matching: find.byIcon(BuzzIcons.house500)),
      findsOneWidget,
    );
    expect(
      find.descendant(of: bar, matching: find.byIcon(BuzzIcons.inbox300)),
      findsOneWidget,
    );
    expect(find.bySemanticsLabel('Activity, unread'), findsOneWidget);
    expect(find.descendant(of: bar, matching: find.byType(Text)), findsNothing);
    final launcherGlass = find.byKey(const ValueKey('quick-actions-ios-glass'));
    expect(launcherGlass, findsOneWidget);
    expect(tester.getSize(launcherGlass), const Size.square(56));
    expect(
      created.where(
        (params) => params['glassTintColor'] == Colors.black.toARGB32(),
      ),
      hasLength(1),
    );
    expect(
      tester
          .widget<Icon>(
            find.descendant(
              of: launcherGlass,
              matching: find.byIcon(BuzzIcons.plus),
            ),
          )
          .color,
      Colors.white,
    );
    expect(
      created.where((params) => params['usesGlass'] == true).length,
      greaterThanOrEqualTo(2),
    );
    expect(
      find.descendant(of: launcherGlass, matching: find.byIcon(BuzzIcons.plus)),
      findsOneWidget,
    );
    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(tester.getSize(launcherGlass).width, greaterThan(56));
    expect(find.text('Create channel').hitTestable(), findsOneWidget);
    expect(find.text('New direct message').hitTestable(), findsOneWidget);
    expect(find.text('Browse channels').hitTestable(), findsOneWidget);
    await tester.tapAt(const Offset(8, 8));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(tester.getSize(launcherGlass), const Size.square(56));
    await tester.tap(find.byTooltip('Activity'));
    await tester.pump(const Duration(milliseconds: 250));
    await tester.pump();
    expect(
      find.descendant(of: bar, matching: find.byIcon(BuzzIcons.inbox500)),
      findsOneWidget,
    );
    expect(find.bySemanticsLabel('Activity, unread'), findsNothing);
    await tester.tap(find.byTooltip('Solo'));
    await tester.pumpAndSettle();
    expect(find.text('Personal workspace'), findsOneWidget);
    expect(find.byType(SearchPage), findsNothing);
    await tester.tap(find.byTooltip('Home'));
    await tester.pumpAndSettle();
    tester
        .widget<FrostedAppBar>(find.byType(FrostedAppBar))
        .nativeActions!
        .firstWhere((action) => action.label == 'Search')
        .onPressed!();
    await tester.pumpAndSettle();
    expect(find.byType(SearchPage), findsOneWidget);
    final searchRoute = ModalRoute.of(tester.element(find.byType(SearchPage)))!;
    expect(searchRoute.allowSnapshotting, isFalse);
    expect(searchRoute.opaque, isTrue);
    expect(searchRoute.transitionDuration, const Duration(milliseconds: 500));
    expect(
      searchRoute.reverseTransitionDuration,
      const Duration(milliseconds: 500),
    );
    Navigator.of(tester.element(find.byType(SearchPage))).pop();
    await tester.pumpAndSettle();
    expect(find.byType(SearchPage), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    debugDefaultTargetPlatformOverride = null;
  });

  for (final reducedMotion in [false, true]) {
    testWidgets('search slides vertically, reduced motion: $reducedMotion', (
      tester,
    ) async {
      await tester.pumpWidget(
        await buildHome(disableAnimations: reducedMotion),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Search'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      final search = find.byType(SearchPage);
      expect(search, findsOneWidget);
      expect(tester.getTopLeft(search).dx, 0);
      expect(tester.getTopLeft(search).dy, reducedMotion ? 0 : greaterThan(0));
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(search).dy, 0);
      expect(ModalRoute.of(tester.element(search))!.opaque, isFalse);
      Navigator.of(tester.element(search)).pop();
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      if (reducedMotion) {
        expect(search, findsNothing);
      } else {
        expect(tester.getTopLeft(search).dy, greaterThan(0));
      }
      await tester.pumpAndSettle();
      expect(search, findsNothing);
    });
  }

  testWidgets('keeps creation beside the tabs and search beside the avatar', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pump();
    final plus = find.byTooltip('Create or start conversation');
    final search = find.byTooltip('Search');
    final avatar = find.byType(ProfileAvatar);
    expect(plus, findsOneWidget);
    expect(search, findsOneWidget);
    expect(tester.getCenter(search).dy, lessThan(tester.getCenter(plus).dy));
    expect(tester.getCenter(search).dy, tester.getCenter(avatar).dy);
    expect(
      tester.getCenter(plus).dy,
      closeTo(tester.getCenter(find.byTooltip('Home')).dy, 0.01),
    );
    await tester.tap(plus);
    await tester.pumpAndSettle();
    expect(find.text('Create channel'), findsOneWidget);
    expect(find.text('New direct message'), findsOneWidget);
    expect(find.text('Browse channels'), findsOneWidget);
  });

  testWidgets('keeps the Buzz backdrop behind the scalable Home screen', (
    tester,
  ) async {
    const gradient = LinearGradient(
      begin: Alignment.topCenter,
      end: Alignment.bottomCenter,
      colors: [Colors.yellow, Colors.blue],
    );
    await tester.pumpWidget(await buildHome(topSectionGradient: gradient));
    await tester.pump();

    final backdrop = find.byKey(
      const ValueKey('home-settings-transition-backdrop'),
    );
    final decoration =
        tester.widget<DecoratedBox>(backdrop).decoration as BoxDecoration;
    expect(decoration.gradient, gradient);
    expect(
      find.byKey(const ValueKey('home-settings-transition-scale')),
      findsOneWidget,
    );
    expect(
      tester
          .widget<Transform>(
            find.byKey(const ValueKey('home-settings-transition-scale')),
          )
          .transform
          .getMaxScaleOnAxis(),
      1,
    );
    expect(
      tester
          .widget<Opacity>(
            find.byKey(const ValueKey('home-settings-transition-opacity')),
          )
          .opacity,
      1,
    );
  });

  testWidgets('keeps Home opaque beneath the Settings transition', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pumpAndSettle();

    await tester.tap(find.byType(ProfileAvatar));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 95));

    double homeOpacity() => tester
        .widget<Opacity>(
          find.byKey(const ValueKey('home-settings-transition-opacity')),
        )
        .opacity;

    expect(homeOpacity(), 1);

    await tester.pumpAndSettle();
    Navigator.of(
      tester.element(
        find.byKey(
          const ValueKey('settings-transition-opacity'),
          skipOffstage: false,
        ),
      ),
    ).pop();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 95));

    expect(homeOpacity(), 1);
  });

  testWidgets('uses one monotonic route animation for Settings and Home', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pumpAndSettle();

    double homeScale() => tester
        .widget<Transform>(
          find.byKey(const ValueKey('home-settings-transition-scale')),
        )
        .transform
        .storage[0];

    await tester.tap(find.byType(ProfileAvatar));
    await tester.pump();

    final settingsTransition = find.byKey(
      const ValueKey('settings-transition-opacity'),
      skipOffstage: false,
    );
    final settingsRoute = ModalRoute.of(tester.element(settingsTransition));

    final entranceScales = <double>[homeScale()];
    final routeValues = <double>[settingsRoute!.animation!.value];
    for (var frame = 0; frame < 15; frame++) {
      await tester.pump(const Duration(milliseconds: 16));
      entranceScales.add(homeScale());
      routeValues.add(settingsRoute.animation!.value);
    }
    expect(entranceScales.first, closeTo(1, 0.000001));
    final reversalFrames = <int>[];
    for (var frame = 1; frame < entranceScales.length; frame++) {
      if (entranceScales[frame] > entranceScales[frame - 1] + 0.000001) {
        reversalFrames.add(frame);
      }
    }
    expect(
      reversalFrames,
      isEmpty,
      reason:
          'Home must scale down in one direction on entrance. '
          'scales=$entranceScales route=$routeValues',
    );
    expect(entranceScales, everyElement(inInclusiveRange(0.97, 1)));
    expect(entranceScales.last, closeTo(0.97, 0.001));

    await tester.pumpAndSettle();
    Navigator.of(tester.element(settingsTransition)).pop();
    await tester.pumpAndSettle();
  });

  testWidgets('gives selection haptics only when the tab changes', (
    tester,
  ) async {
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            hapticCalls.add(call);
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(await buildHome());
    await tester.pump();

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    expect(hapticCalls, isEmpty);

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));

    await tester.tap(find.byTooltip('Solo'));
    await tester.pump();
    expect(hapticCalls, hasLength(2));
  });

  testWidgets('gives a light impact when the Home quick action is pressed', (
    tester,
  ) async {
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            hapticCalls.add(call);
          }
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(await buildHome());
    await tester.pump();

    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pump();

    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.lightImpact');
  });

  testWidgets('badges the Inbox tab when it has unread rows', (tester) async {
    await tester.pumpWidget(await buildHome(unreadInboxCount: 1));
    await tester.pump();

    expect(
      find.byKey(const ValueKey('activity-tab-unread-dot')),
      findsOneWidget,
    );
    final badge = tester.widget<Container>(
      find.byKey(const ValueKey('activity-tab-unread-dot')),
    );
    expect(badge.constraints?.maxWidth, 12);
    expect(badge.constraints?.maxHeight, 12);
    expect(find.bySemanticsLabel('Activity, unread'), findsOneWidget);
    AnimatedScale unreadDotScale() => tester.widget<AnimatedScale>(
      find.byKey(const ValueKey('activity-tab-unread-dot-scale')),
    );
    expect(unreadDotScale().scale, 1);
    expect(unreadDotScale().alignment, const Alignment(-0.5, 0.5));
    expect(unreadDotScale().duration, const Duration(milliseconds: 220));

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();

    expect(
      find.byKey(const ValueKey('activity-tab-unread-dot')),
      findsOneWidget,
    );
    expect(unreadDotScale().scale, 0);
    expect(find.bySemanticsLabel('Activity, unread'), findsNothing);

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();

    expect(unreadDotScale().scale, 1);
  });

  testWidgets('fades and slides tab content in the selected direction', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    Transform bodyTransform() => tester.widget<Transform>(
      find.byKey(const ValueKey('frosted-scaffold-body-transition-transform')),
    );
    Opacity bodyOpacity() => tester.widget<Opacity>(
      find.byKey(const ValueKey('frosted-scaffold-body-transition-opacity')),
    );
    Transform appBarTransform() => tester.widget<Transform>(
      find.byKey(
        const ValueKey('frosted-app-bar-content-transition-transform'),
      ),
    );
    Opacity appBarOpacity() => tester.widget<Opacity>(
      find.byKey(const ValueKey('frosted-app-bar-content-transition-opacity')),
    );
    double bodyOffset() => bodyTransform().transform.getTranslation().x;
    double appBarOffset() => appBarTransform().transform.getTranslation().x;

    expect(bodyOffset(), closeTo(0, 0.001));
    expect(appBarOffset(), closeTo(0, 0.001));
    expect(bodyOpacity().opacity, closeTo(1, 0.001));
    expect(appBarOpacity().opacity, closeTo(1, 0.001));

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();

    expect(bodyOffset(), closeTo(24, 0.001));
    expect(appBarOffset(), closeTo(24, 0.001));
    expect(bodyOpacity().opacity, closeTo(0, 0.001));
    expect(appBarOpacity().opacity, closeTo(0, 0.001));
    expect(
      find.descendant(
        of: find.byKey(const ValueKey('frosted-app-bar-background')),
        matching: find.byKey(
          const ValueKey('frosted-app-bar-content-transition-transform'),
        ),
      ),
      findsOneWidget,
    );

    await tester.pump(const Duration(milliseconds: 120));

    expect(bodyOffset(), inExclusiveRange(0, 24));
    expect(appBarOffset(), inExclusiveRange(0, 24));
    expect(bodyOpacity().opacity, inExclusiveRange(0, 1));
    expect(appBarOpacity().opacity, inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Home'));
    await tester.pump();

    expect(bodyOffset(), closeTo(-24, 0.001));
    expect(appBarOffset(), closeTo(-24, 0.001));
    expect(bodyOpacity().opacity, closeTo(0, 0.001));
    expect(appBarOpacity().opacity, closeTo(0, 0.001));

    await tester.pumpAndSettle();
    expect(bodyOffset(), closeTo(0, 0.001));
    expect(appBarOffset(), closeTo(0, 0.001));
    expect(bodyOpacity().opacity, closeTo(1, 0.001));
    expect(appBarOpacity().opacity, closeTo(1, 0.001));
  });

  testWidgets('switches tab content instantly with reduced motion', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome(disableAnimations: true));
    await tester.pump();

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();

    final bodyTransform = tester.widget<Transform>(
      find.byKey(const ValueKey('frosted-scaffold-body-transition-transform')),
    );
    final bodyOpacity = tester.widget<Opacity>(
      find.byKey(const ValueKey('frosted-scaffold-body-transition-opacity')),
    );
    final appBarTransform = tester.widget<Transform>(
      find.byKey(
        const ValueKey('frosted-app-bar-content-transition-transform'),
      ),
    );
    final appBarOpacity = tester.widget<Opacity>(
      find.byKey(const ValueKey('frosted-app-bar-content-transition-opacity')),
    );
    expect(bodyTransform.transform.getTranslation().x, closeTo(0, 0.001));
    expect(appBarTransform.transform.getTranslation().x, closeTo(0, 0.001));
    expect(bodyOpacity.opacity, closeTo(1, 0.001));
    expect(appBarOpacity.opacity, closeTo(1, 0.001));
  });
  testWidgets('scales and fades the quick action as tabs change', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome());
    await tester.pump();

    double scale() => tester
        .widget<Transform>(find.byKey(const Key('channel-quick-actions-scale')))
        .transform
        .storage
        .first;
    double opacity() => tester
        .widget<Opacity>(find.byKey(const Key('channel-quick-actions-opacity')))
        .opacity;

    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));

    await tester.tap(find.byTooltip('Activity'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(0.8, 0.001));
    expect(opacity(), closeTo(0, 0.001));

    await tester.tap(find.byTooltip('Home'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));

    expect(scale(), inExclusiveRange(0.8, 1));
    expect(opacity(), inExclusiveRange(0, 1));

    await tester.pumpAndSettle();
    expect(scale(), closeTo(1, 0.001));
    expect(opacity(), closeTo(1, 0.001));
  });
}

Widget _buildSettingsPage(BuildContext context) => const SizedBox.shrink();
