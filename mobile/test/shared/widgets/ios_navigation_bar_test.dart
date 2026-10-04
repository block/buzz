import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  for (final reverse in [false, true]) {
    testWidgets('initial material tracks content above reverse=$reverse', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final controller = ScrollController();
      addTearDown(controller.dispose);
      ValueListenable<double>? offset;
      Widget page(int count) => _testApp(
        home: FrostedScaffold(
          appBar: const FrostedAppBar(title: Text('Conversation')),
          body: Builder(
            builder: (context) {
              offset = IosNavigationScrollScope.maybeOf(context);
              return ListView(
                controller: controller,
                reverse: reverse,
                children: [
                  for (var i = 0; i < count; i++)
                    SizedBox(height: 60, child: Text('Message $i')),
                ],
              );
            },
          ),
        ),
      );
      await tester.pumpWidget(page(40));
      await tester.pumpAndSettle();
      expect(offset!.value, reverse ? greaterThan(0) : 0);
      controller.jumpTo(controller.position.maxScrollExtent);
      await tester.pumpAndSettle();
      expect(offset!.value, reverse ? 0 : greaterThan(0));
      await tester.pumpWidget(page(1));
      await tester.pumpAndSettle();
      expect(offset!.value, 0);
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    });
  }

  testWidgets('native material follows live theme surface changes', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/ios_navigation_bar/845');
    final configurations = <Map<Object?, Object?>>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      if (call.method == 'configure') configurations.add(call.arguments as Map);
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    Widget app(Color surface) => ProviderScope(
      child: MaterialApp(
        theme: ThemeData(
          colorScheme: ColorScheme.fromSeed(
            seedColor: surface,
          ).copyWith(surface: surface),
        ),
        home: const Scaffold(
          body: IosNavigationBar(title: 'general', subtitle: '3 members'),
        ),
      ),
    );
    await tester.pumpWidget(app(const Color(0xFFFFEEDD)));
    final view = tester.widget<UiKitView>(find.byType(UiKitView));
    expect(view.creationParams, containsPair('background', 0xFFFFEEDD));
    view.onPlatformViewCreated!(845);
    await tester.pump();
    await tester.pumpWidget(app(const Color(0xFF223344)));
    await tester.pumpAndSettle();
    expect(configurations.last['background'], 0xFF223344);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('larger native metrics keep a deeply scrolled title collapsed', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/ios_navigation_bar/843');
    final calls = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      calls.add(call);
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    await tester.pumpWidget(
      _testApp(
        home: FrostedScaffold(
          nativePinnedBody: true,
          appBar: const FrostedAppBar(
            title: Text('Search'),
            nativeLargeTitle: true,
          ),
          body: ListView(
            children: [
              for (var i = 0; i < 40; i++)
                SizedBox(height: 60, child: Text('Row $i')),
            ],
          ),
        ),
      ),
    );
    tester.widget<UiKitView>(find.byType(UiKitView)).onPlatformViewCreated!(
      843,
    );
    await tester.pump();
    await tester.drag(find.byType(ListView), const Offset(0, -250));
    await tester.pumpAndSettle();
    expect(calls.where((call) => call.method == 'scroll').last.arguments, 52);
    final bodyTop = tester.getTopLeft(find.byType(ListView)).dy;
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      channel.name,
      channel.codec.encodeMethodCall(
        const MethodCall('metrics', {
          'compactHeight': 44.0,
          'expandedHeight': 124.0,
        }),
      ),
      (_) {},
    );
    await tester.pump();
    await tester.pump();
    expect(calls.where((call) => call.method == 'scroll').last.arguments, 80);
    expect(tester.getSize(find.byType(UiKitView)).height, 44);
    expect(tester.getTopLeft(find.byType(ListView)).dy, bodyTop);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'iOS routes use UIKit and route native menus to current callbacks',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const channel = MethodChannel('${IosNavigationBar.viewType}/42');
      final calls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        calls.add(call);
        return null;
      });
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          channel,
          null,
        ),
      );
      var selected = 0;
      var callbackVersion = 1;
      Widget page() {
        final version = callbackVersion;
        return _testApp(
          theme: AppTheme.light(),
          home: FrostedScaffold(
            appBar: FrostedAppBar(
              nativeTitle: 'Activity',
              nativeLargeTitle: true,
              nativeActions: [
                IosNavigationAction(
                  label: 'Filter',
                  symbol: 'line.3.horizontal.decrease',
                  children: [
                    IosNavigationAction(
                      label: 'Mentions',
                      onPressed: () => selected = version,
                    ),
                  ],
                ),
              ],
            ),
            body: ListView(
              children: [
                for (var i = 0; i < 40; i++)
                  SizedBox(height: 60, child: Text('Item $i')),
              ],
            ),
          ),
        );
      }

      await tester.pumpWidget(page());
      final view = tester.widget<UiKitView>(find.byType(UiKitView));
      expect(view.viewType, IosNavigationBar.viewType);
      expect(view.creationParams, containsPair('largeTitle', true));
      view.onPlatformViewCreated!(42);
      await tester.pump();
      callbackVersion = 2;
      await tester.pumpWidget(page());
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        channel.name,
        channel.codec.encodeMethodCall(const MethodCall('action', '0.0')),
        (_) {},
      );
      expect(selected, 2);
      await tester.drag(find.byType(ListView), const Offset(0, -180));
      await tester.pumpAndSettle();
      expect(calls.where((call) => call.method == 'scroll').last.arguments, 52);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('native back button respects the Flutter route stack', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final navigator = GlobalKey<NavigatorState>();
    await tester.pumpWidget(
      _testApp(
        navigatorKey: navigator,
        home: const Scaffold(body: Text('Root')),
      ),
    );
    navigator.currentState!.push(
      MaterialPageRoute<void>(
        builder: (_) => const FrostedScaffold(
          appBar: FrostedAppBar(title: Text('Theme')),
          body: SizedBox.expand(),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final native = tester.widget<IosNavigationBar>(
      find.byType(IosNavigationBar),
    );
    expect(native.title, 'Theme');
    expect(native.onBack, isNotNull);
    native.onBack!();
    await tester.pumpAndSettle();
    expect(find.text('Root'), findsOneWidget);
    expect(find.byType(UiKitView), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('UIKit measurements resize the bar and page spacing together', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('${IosNavigationBar.viewType}/91');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      channel,
      (_) async => null,
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    await tester.pumpWidget(
      _testApp(
        home: Builder(
          builder: (context) => FrostedScaffold(
            appBar: const FrostedAppBar(
              title: Text('Home'),
              nativeLargeTitle: true,
            ),
            body: ListView(
              children: [
                SizedBox(
                  key: const ValueKey('body-inset'),
                  height: frostedAppBarHeight(context, nativeLargeTitle: true),
                ),
              ],
            ),
          ),
        ),
      ),
    );
    final view = tester.widget<UiKitView>(find.byType(UiKitView));
    view.onPlatformViewCreated!(91);
    await tester.pump();
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      channel.name,
      channel.codec.encodeMethodCall(
        const MethodCall('metrics', {
          'compactHeight': 56.0,
          'expandedHeight': 112.0,
        }),
      ),
      (_) {},
    );
    await tester.pump();
    expect(tester.getSize(find.byType(UiKitView)).height, 112);
    expect(
      tester.getSize(find.byKey(const ValueKey('body-inset'))).height,
      112,
    );
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('loading avatars use initials instead of navigation symbols', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      _testApp(
        home: const IosNavigationBar(
          title: 'Home',
          leading: IosNavigationAction(
            label: 'Community',
            symbol: 'building.2',
            avatarInitial: 'K',
            avatarIdentity: 'community-one',
          ),
          actions: [
            IosNavigationAction(
              label: 'Profile',
              symbol: 'person.crop.circle',
              avatarInitial: 'A',
              avatarIdentity: 'profile-one',
            ),
          ],
        ),
      ),
    );
    final params =
        tester.widget<UiKitView>(find.byType(UiKitView)).creationParams! as Map;
    expect((params['leading'] as Map)['symbol'], isNull);
    expect((params['leading'] as Map)['avatarInitial'], 'K');
    final profile = (params['actions'] as List).single as Map;
    expect(profile['symbol'], isNull);
    expect(profile['avatarInitial'], 'A');
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'retains decoded avatars on theme refresh but isolates identities',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final messenger = tester.binding.defaultBinaryMessenger;
      final channels = <MethodChannel>[];
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
        call,
      ) async {
        if (call.method == 'create') {
          final id = (call.arguments as Map)['id'];
          final channel = MethodChannel('${IosNavigationBar.viewType}/$id');
          channels.add(channel);
          messenger.setMockMethodCallHandler(channel, (_) async => null);
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
        for (final channel in channels) {
          messenger.setMockMethodCallHandler(channel, null);
        }
      });
      Widget page(String identity, ThemeData theme) => _testApp(
        theme: theme,
        home: IosNavigationBar(
          title: 'Home',
          leading: IosNavigationAction(
            label: 'Profile',
            avatarInitial: 'K',
            avatarIdentity: identity,
          ),
        ),
      );
      Map avatar() =>
          (tester.widget<UiKitView>(find.byType(UiKitView)).creationParams!
                  as Map)['leading']
              as Map;
      await tester.pumpWidget(page('one', AppTheme.light()));
      await tester.runAsync(() async {
        for (
          var attempt = 0;
          attempt < 100 && avatar()['imageData'] == null;
          attempt++
        ) {
          await Future<void>.delayed(const Duration(milliseconds: 10));
          await tester.pump();
        }
      });
      final image = avatar()['imageData'];
      expect(image, isNotNull);
      await tester.pumpWidget(page('one', AppTheme.dark()));
      expect(avatar()['imageData'], image);
      await tester.pumpWidget(page('two', AppTheme.dark()));
      expect(avatar()['imageData'], isNull);
      expect(avatar()['symbol'], isNull);
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('Android retains its Flutter header', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      _testApp(
        theme: AppTheme.light(),
        home: const FrostedScaffold(
          appBar: FrostedAppBar(title: Text('Settings')),
          body: SizedBox.expand(),
        ),
      ),
    );
    expect(find.byType(UiKitView), findsNothing);
    expect(find.text('Settings'), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });
}

Widget _testApp({
  ThemeData? theme,
  required Widget home,
  GlobalKey<NavigatorState>? navigatorKey,
}) => ProviderScope(
  child: MaterialApp(
    theme: theme,
    home: home,
    navigatorKey: navigatorKey,
    builder: (context, child) => IosNavigationMetricsHost(child: child!),
  ),
);
