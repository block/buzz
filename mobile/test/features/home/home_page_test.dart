import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/concentric_sheet_surface.dart';
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
    ValueChanged<ValueNotifier<VoidCallback?>>? onNewChatAction,
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
          mePageBuilder: (_, action) {
            onNewChatAction?.call(action);
            return const FrostedScaffold(
              appBar: FrostedAppBar(title: Text('Solo')),
              body: Center(child: Text('Personal workspace')),
            );
          },
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
    final initialHeaders = <Map<Object?, Object?>>[];
    messenger.setMockMethodCallHandler(support, (call) async => true);
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'create') {
        final args = call.arguments as Map<Object?, Object?>;
        if (args['viewType'] == 'buzz/ios_navigation_bar') {
          initialHeaders.add(
            const StandardMessageCodec().decodeMessage(
                  ByteData.sublistView(args['params'] as Uint8List),
                )
                as Map<Object?, Object?>,
          );
        }
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
    // Inspect the creation payload, not a later configure call after PNG rendering.
    final initialActions = initialHeaders.first['actions'] as List;
    for (final entry in {
      'Search': 'magnifyingglass',
      'Inbox': 'tray',
    }.entries) {
      final action = initialActions.cast<Map>().firstWhere(
        (action) => action['label'] == entry.key,
      );
      expect(action['iconData'], isNull);
      expect(action['symbol'], entry.value);
    }
    final bar = find.byKey(const ValueKey('home-ios-glass-tabs'));
    expect(bar, findsOneWidget);
    expect(tester.getSize(bar), const Size(148, 56));
    expect(created.where((params) => params['usesGlass'] == true), isNotEmpty);
    expect(
      find.descendant(of: bar, matching: find.byIcon(BuzzIcons.house500)),
      findsOneWidget,
    );
    expect(
      find.descendant(of: bar, matching: find.byIcon(BuzzIcons.inbox300)),
      findsNothing,
    );
    final headerActions = tester
        .widget<FrostedAppBar>(find.byType(FrostedAppBar))
        .nativeActions!;
    expect(headerActions.map((action) => action.label), [
      'Search',
      'Inbox',
      'Settings',
    ]);
    expect(headerActions.map((action) => action.sharesBackground), [
      true,
      true,
      false,
    ]);
    expect(headerActions[0].icon, BuzzIcons.search300);
    expect(headerActions[1].icon, BuzzIcons.inbox300);
    expect(headerActions[1].activityColor, isNotNull);
    expect(headerActions[1].activityLabel, 'Unread');
    expect(find.descendant(of: bar, matching: find.byType(Text)), findsNothing);
    final launcherGlass = find.byKey(const ValueKey('quick-actions-ios-glass'));
    expect(launcherGlass, findsOneWidget);
    expect(tester.getSize(launcherGlass), const Size.square(56));
    final launcherSurface = tester.widget<ConcentricSheetSurface>(
      launcherGlass,
    );
    expect(launcherSurface.usesGlass, isTrue);
    expect(launcherSurface.glassTintColor, isNull);
    expect(
      tester
          .widget<Icon>(
            find.descendant(
              of: launcherGlass,
              matching: find.byIcon(BuzzIcons.plus),
            ),
          )
          .color,
      Theme.of(tester.element(launcherGlass)).colorScheme.onSurface,
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
    expect(
      find.byKey(const Key('quick-action-browse-channels-card')),
      findsNothing,
    );
    await tester.tapAt(const Offset(8, 8));
    await tester.pump();
    await tester.pump(const Duration(seconds: 1));
    expect(tester.getSize(launcherGlass), const Size.square(56));
    headerActions[1].onPressed!();
    await tester.pumpAndSettle();
    expect(find.byType(ActivityPage), findsOneWidget);
    final inboxHeader = tester.widget<FrostedAppBar>(
      find.byType(FrostedAppBar),
    );
    expect(inboxHeader.nativeTitle, 'Inbox');
    expect(inboxHeader.automaticallyImplyLeading, isFalse);
    expect(inboxHeader.nativeLeading?.symbol, 'xmark');
    inboxHeader.nativeLeading!.onPressed!();
    await tester.pumpAndSettle();
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
    expect(
      tester.getTopLeft(find.byTooltip('Home')).dx,
      Grid.gutter + Grid.half,
    );
    expect(
      tester.getBottomRight(plus).dx,
      tester.view.physicalSize.width / tester.view.devicePixelRatio -
          Grid.gutter,
    );
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
    expect(
      find.byKey(const Key('quick-action-browse-channels-card')),
      findsNothing,
    );
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

  testWidgets('Settings rises from below without scaling Home', (tester) async {
    await tester.pumpWidget(await buildHome());
    await tester.pumpAndSettle();
    await tester.tap(find.byType(ProfileAvatar));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    final transform = tester.widget<Transform>(
      find.byKey(const ValueKey('home-settings-transition-scale')),
    );
    expect(transform.transform.getMaxScaleOnAxis(), 1);
    await tester.pumpAndSettle();
    tester.state<NavigatorState>(find.byType(Navigator).first).pop();
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

    await tester.tap(find.byTooltip('Solo'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.selectionClick');

    await tester.tap(find.byTooltip('Solo'));
    await tester.pump();
    expect(hapticCalls, hasLength(1));

    await tester.tap(find.byTooltip('Home'));
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

  testWidgets('opens Inbox from its unread header button and returns Home', (
    tester,
  ) async {
    await tester.pumpWidget(await buildHome(unreadInboxCount: 1));
    await tester.pumpAndSettle();
    final badge = find.byKey(const ValueKey('inbox-header-badge'));
    expect(tester.widget<Badge>(badge).isLabelVisible, isTrue);
    expect(find.bySemanticsLabel('Inbox, unread'), findsOneWidget);
    expect(find.byTooltip('Activity'), findsNothing);
    final search = tester.getCenter(find.byTooltip('Search'));
    final inbox = tester.getCenter(find.byTooltip('Inbox'));
    final avatar = tester.getCenter(find.byType(ProfileAvatar));
    expect(inbox.dx, greaterThan(search.dx));
    expect(inbox.dx, lessThan(avatar.dx));
    expect(inbox.dy, search.dy);
    await tester.tap(find.byTooltip('Inbox'));
    await tester.pumpAndSettle();
    expect(find.byType(ActivityPage), findsOneWidget);
    expect(find.text('Inbox'), findsOneWidget);
    await tester.tap(find.byTooltip('Close inbox'));
    await tester.pumpAndSettle();
    expect(find.byType(ActivityPage), findsNothing);
    expect(find.byTooltip('Search'), findsOneWidget);
    await tester.pumpWidget(await buildHome());
    await tester.pumpAndSettle();
    expect(tester.widget<Badge>(badge).isLabelVisible, isFalse);
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

    await tester.tap(find.byTooltip('Solo'));
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

    await tester.tap(find.byTooltip('Solo'));
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
  testWidgets('Solo compose stays anchored through keyboard dismissal', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    tester.view.padding = const FakeViewPadding(bottom: 34);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(await buildHome());
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Solo'));
    await tester.pumpAndSettle();
    final compose = find.byTooltip('New chat');
    final restingPosition = tester.getCenter(compose);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    tester.view.padding = FakeViewPadding.zero;
    await tester.pumpAndSettle();
    expect(tester.getCenter(compose), restingPosition);
    tester.view.viewInsets = FakeViewPadding.zero;
    tester.view.padding = const FakeViewPadding(bottom: 34);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 80));
    expect(tester.getCenter(compose), restingPosition);
    await tester.pumpAndSettle();
    expect(tester.getCenter(compose), restingPosition);
  });

  testWidgets('keeps the plus opposite the tabs and opens Solo New chat', (
    tester,
  ) async {
    ValueNotifier<VoidCallback?>? action;
    var newChats = 0;
    await tester.pumpWidget(
      await buildHome(onNewChatAction: (value) => action = value),
    );
    await tester.pumpAndSettle();
    final homePlusPosition = tester.getCenter(
      find.byTooltip('Create or start conversation'),
    );
    await tester.tap(find.byTooltip('Solo'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Create or start conversation'), findsNothing);
    expect(tester.getCenter(find.byTooltip('New chat')), homePlusPosition);
    expect(find.byIcon(BuzzIcons.edit), findsOneWidget);
    action!.value = () => newChats++;
    await tester.pump();
    await tester.tap(find.byTooltip('New chat'));
    await tester.pumpAndSettle();
    expect(newChats, 1);
    expect(find.text('Create channel').hitTestable(), findsNothing);
    await tester.tap(find.byTooltip('Home'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pumpAndSettle();
    expect(find.text('Create channel'), findsOneWidget);
    expect(newChats, 1);
    await tester.tap(find.byTooltip('Solo'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Home'));
    await tester.pumpAndSettle();
    expect(find.text('Create channel').hitTestable(), findsNothing);
    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pumpAndSettle();
    expect(find.text('Create channel').hitTestable(), findsOneWidget);
  });
}

Widget _buildSettingsPage(BuildContext context) => const SizedBox.shrink();
