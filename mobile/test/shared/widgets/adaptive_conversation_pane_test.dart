import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:buzz/shared/widgets/text_editor.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/modal_presentation.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('reopening a deep link resets its route after thread Back', (
    tester,
  ) async {
    final controller = ConversationPaneController();
    addTearDown(controller.dispose);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1000, 800);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          home: AdaptiveConversationPane(
            controller: controller,
            list: Builder(
              builder: (context) => TextButton(
                onPressed: () => openConversationPane(
                  context,
                  'activity-item',
                  (_) => const _DeepLink(),
                  reopen: true,
                ),
                child: const Text('Open activity'),
              ),
            ),
          ),
        ),
      ),
    );
    for (var attempt = 0; attempt < 2; attempt++) {
      await tester.tap(find.text('Open activity'));
      await tester.pumpAndSettle();
      expect(find.text('Deep-linked thread'), findsOneWidget);
      await tester.tap(find.byType(BackButton));
      await tester.pumpAndSettle();
      expect(find.text('Selected channel'), findsOneWidget);
      expect(controller.selectedId, 'activity-item');
      expect(
        ConversationPaneScope.isBaseRouteOf(
          tester.element(find.text('Selected channel')),
        ),
        isTrue,
      );
    }
  });

  testWidgets('phones stay compact while tablets and Duo retain panes', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
      debugDefaultTargetPlatformOverride = platform;
      for (final scenario in [
        (size: const Size(402, 874), duo: false, top: 24.0, wide: false),
        (size: const Size(874, 402), duo: false, top: 24.0, wide: false),
        (size: const Size(1024, 768), duo: false, top: 24.0, wide: true),
        (size: const Size(768, 1024), duo: false, top: 24.0, wide: true),
        (size: const Size(874, 402), duo: true, top: 0.0, wide: true),
        (
          size: const Size(669, 951),
          duo: false,
          top: 82.0,
          wide: platform != TargetPlatform.iOS,
        ),
      ]) {
        tester.view.physicalSize = scenario.size;
        tester.view.viewPadding = FakeViewPadding(top: scenario.top);
        await tester.pumpWidget(
          MaterialApp(
            home: Builder(
              builder: (context) {
                final wide = usesConversationPanes(
                  context,
                  strip: scenario.duo
                      ? const FoldStrip(
                          side: FoldStripSide.right,
                          rect: Rect.fromLTWH(798, 0, 76, 402),
                          topClearance: 0,
                          bottomClearance: 0,
                          isClearanceEstimated: true,
                        )
                      : null,
                );
                return Text(wide ? 'Split panes' : 'Single pane');
              },
            ),
          ),
        );
        expect(
          find.text(scenario.wide ? 'Split panes' : 'Single pane'),
          findsOneWidget,
        );
      }
    }
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'detail surface reaches rail and sheet covers the whole workspace',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(951, 669);
      tester.view.padding = const FakeViewPadding(right: 76);
      tester.view.viewPadding = const FakeViewPadding(right: 76);
      addTearDown(tester.view.reset);
      final controller = ConversationPaneController();
      addTearDown(controller.dispose);
      controller.open(
        'settings',
        (context) => FrostedScaffold(
          key: const ValueKey('detail-surface'),
          appBar: const FrostedAppBar(title: Text('Channel settings')),
          body: Center(
            child: TextButton(
              onPressed: () => showBuzzModalBottomSheet<void>(
                context: context,
                title: 'Channel actions',
                builder: (_) => const SizedBox(
                  height: 200,
                  child: Center(child: Text('Sheet content')),
                ),
              ),
              child: const Text('Open sheet'),
            ),
          ),
        ),
      );
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: IosNavigationMetricsHost(
              child: AdaptiveConversationPane(
                controller: controller,
                strip: const FoldStrip(
                  side: FoldStripSide.right,
                  rect: Rect.fromLTWH(875, 0, 76, 669),
                  topClearance: 110,
                  bottomClearance: 0,
                  isClearanceEstimated: true,
                ),
                list: const ColoredBox(
                  color: Colors.white,
                  child: Text('Sidebar'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        tester.getRect(find.byKey(const ValueKey('detail-surface'))).right,
        951,
      );
      expect(
        tester.getCenter(find.text('Open sheet')).dx,
        closeTo((875 / 3 + 1 + 875) / 2, 1),
      );
      final flatLeft = tester
          .getRect(find.byKey(const ValueKey('detail-surface')))
          .left;
      for (final division in [
        <double>[470, 0, 12, 669],
        null,
      ]) {
        IosNavigationMetrics.updateRail(tester.element(find.text('Sidebar')), {
          'centerX': 920,
          'windowWidth': 951,
          'windowHeight': 669,
          'bookDivision': division,
        });
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 120));
        final moving = tester.getRect(
          find.byKey(const ValueKey('detail-surface')),
        );
        expect(moving.left, greaterThan(flatLeft));
        expect(moving.left, lessThan(476));
        expect(moving.right, 951);
        await tester.pumpAndSettle();
        final detail = tester.getRect(
          find.byKey(const ValueKey('detail-surface')),
        );
        expect(detail.left, division == null ? flatLeft : 476);
        expect(detail.right, 951);
        if (division != null) {
          expect(tester.getRect(find.text('Sidebar')).right, detail.left);
          expect(
            tester.getCenter(find.text('Open sheet')).dx,
            closeTo((482 + 875) / 2, 1),
          );
        }
        expect(controller.selectedId, 'settings');
        expect(find.text('Open sheet'), findsOneWidget);
      }
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      addTearDown(
        tester.platformDispatcher.clearAccessibilityFeaturesTestValue,
      );
      await tester.pump();
      IosNavigationMetrics.updateRail(tester.element(find.text('Sidebar')), {
        'centerX': 920,
        'windowWidth': 951,
        'windowHeight': 669,
        'bookDivision': <double>[470, 0, 12, 669],
      });
      await tester.pump();
      expect(
        tester.getRect(find.byKey(const ValueKey('detail-surface'))).left,
        476,
      );
      IosNavigationMetrics.updateRail(tester.element(find.text('Sidebar')), {
        'centerX': 920,
        'windowWidth': 951,
        'windowHeight': 669,
      });
      await tester.pump();
      expect(
        tester.getRect(find.byKey(const ValueKey('detail-surface'))).left,
        flatLeft,
      );
      await tester.tap(find.text('Open sheet'));
      await tester.pumpAndSettle();
      expect(tester.getRect(find.byType(BottomSheet)).left, 0);
      expect(tester.getRect(find.byType(BottomSheet)).right, 951);
      expect(tester.getRect(find.text('Sheet content')).right, lessThan(875));
      final close = find.byKey(
        const ValueKey('duo-toolbar-Close Channel actions'),
      );
      expect(close, findsOneWidget);
      expect(
        find.byKey(const ValueKey('buzz-sheet-ios-glass-close')),
        findsNothing,
      );
      expect(tester.getRect(close).center.dx, greaterThan(875));
      final railBar = tester.widget<IosNavigationBar>(
        find.descendant(of: close, matching: find.byType(IosNavigationBar)),
      );
      railBar.leading!.onPressed!();
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('detail-surface')), findsOneWidget);
      expect(controller.selectedId, 'settings');
      const nativeEditor = MethodChannel('buzz/profile_text_editor');
      final nativeTitles = <String>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        nativeEditor,
        (call) async {
          nativeTitles.add((call.arguments as Map)['title'] as String);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          nativeEditor,
          null,
        ),
      );
      for (final title in ['Display name', 'Profile description']) {
        showTextEditor(
          context: tester.element(find.text('Open sheet')),
          title: title,
          initialValue: 'Original',
          hintText: title,
          onSave: (_) async {},
        );
        await tester.pumpAndSettle();
        expect(find.byType(TextField), findsNothing);
      }
      expect(nativeTitles, ['Display name', 'Profile description']);
      expect(controller.selectedId, 'settings');
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );
  testWidgets(
    'result destination keeps its list and closes detail before workspace',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(900, 700);
      addTearDown(tester.view.reset);
      final navigator = GlobalKey<NavigatorState>();
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            navigatorKey: navigator,
            theme: AppTheme.light(),
            home: const Scaffold(body: Text('Home')),
          ),
        ),
      );
      navigator.currentState!.push(
        MaterialPageRoute<void>(
          builder: (_) => AdaptiveConversationDestination(
            child: Builder(
              builder: (context) => Scaffold(
                body: Column(
                  children: [
                    const TextField(key: ValueKey('result-query')),
                    TextButton(
                      onPressed: () => openConversationPane(
                        context,
                        'result',
                        (_) => const _Conversation(),
                      ),
                      child: const Text('Open result'),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('result-query')),
        'retained query',
      );
      await tester.tap(find.text('Open result'));
      await tester.pumpAndSettle();
      expect(find.text('retained query'), findsOneWidget);
      expect(find.byType(TextField), findsNWidgets(2));
      tester.view.physicalSize = const Size(400, 700);
      await tester.pumpAndSettle();
      expect(find.text('Open result'), findsNothing);
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.text('Open result'), findsOneWidget);
      expect(find.text('retained query'), findsOneWidget);
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.text('Home'), findsOneWidget);
    },
  );

  testWidgets(
    'selection, draft and thread survive compact and regular layouts',
    (tester) async {
      final controller = ConversationPaneController();
      final active = ValueNotifier(true);
      addTearDown(active.dispose);
      addTearDown(controller.dispose);
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(900, 700);
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(
              body: ValueListenableBuilder<bool>(
                valueListenable: active,
                builder: (context, isActive, _) => AdaptiveConversationPane(
                  active: isActive,
                  controller: controller,
                  list: Builder(
                    builder: (context) => Column(
                      children: [
                        TextButton(
                          onPressed: () => ConversationPaneScope.maybeOf(
                            context,
                          )!.open('one', (_) => const _Conversation()),
                          child: const Text('First chat'),
                        ),
                        TextButton(
                          onPressed: () =>
                              ConversationPaneScope.maybeOf(context)!.open(
                                'two',
                                (_) => const Scaffold(
                                  body: Text('Second conversation'),
                                ),
                              ),
                          child: const Text('Second chat'),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('First chat'));
      await tester.pumpAndSettle();
      expect(
        tester.getCenter(find.text('First chat')).dx,
        lessThan(tester.getCenter(find.byType(TextField)).dx),
      );
      await tester.enterText(find.byType(TextField), 'Keep my draft');
      active.value = false;
      await tester.pumpAndSettle();
      expect(find.text('Visible: false'), findsOneWidget);
      active.value = true;
      await tester.pumpAndSettle();
      expect(find.text('Visible: true'), findsOneWidget);
      tester.view.physicalSize = const Size(390, 700);
      await tester.pumpAndSettle();
      expect(find.text('First chat'), findsNothing);
      expect(find.text('Keep my draft'), findsOneWidget);
      await tester.tap(find.text('Thread'));
      await tester.pumpAndSettle();
      expect(find.text('Thread content'), findsOneWidget);
      tester.view.physicalSize = const Size(900, 700);
      await tester.pumpAndSettle();
      expect(find.text('First chat'), findsOneWidget);
      expect(find.text('Thread content'), findsOneWidget);
      await tester.tap(find.byType(BackButton));
      await tester.pumpAndSettle();
      expect(find.text('Keep my draft'), findsOneWidget);
      await tester.tap(find.text('Second chat'));
      await tester.pumpAndSettle();
      expect(find.text('Second conversation'), findsOneWidget);
      expect(controller.selectedId, 'two');
      tester.view.physicalSize = const Size(390, 700);
      await tester.pumpAndSettle();
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(controller.selectedId, isNull);
      expect(find.text('First chat'), findsOneWidget);
      tester.view.physicalSize = const Size(900, 700);
      await tester.pumpAndSettle();
      final placeholder = find.text('Select a conversation');
      expect(placeholder, findsOneWidget);
      expect(
        find.ancestor(of: placeholder, matching: find.byType(Material)),
        findsWidgets,
      );
      expect(
        tester.widget<Text>(placeholder).style!.decoration,
        TextDecoration.none,
      );
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('iPhone back swipe and back button reveal the chat list', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(402, 874);
    addTearDown(tester.view.reset);
    final controller = ConversationPaneController();
    addTearDown(controller.dispose);
    var listTaps = 0;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          home: AdaptiveConversationPane(
            controller: controller,
            list: Material(
              child: Center(
                child: TextButton(
                  onPressed: () => listTaps++,
                  child: const Text('Chat list'),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    final list = find.text('Chat list');
    final placeholder = find.text('Select a conversation');

    controller.open('chat', (_) => const _Conversation());
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(list, findsOneWidget, reason: 'the list stays beneath a push');
    await tester.pumpAndSettle();
    expect(list, findsNothing, reason: 'a settled chat covers the list');

    final swipe = await tester.startGesture(const Offset(5, 400));
    await swipe.moveBy(const Offset(100, 0));
    await tester.pump();
    await swipe.moveBy(const Offset(60, 0));
    await tester.pump();
    expect(list, findsOneWidget);
    expect(placeholder, findsNothing);
    expect(
      tester.getSemantics(list.first),
      isNot(matchesSemantics(label: 'Chat list')),
      reason: 'the covered list is not a screen-reader stop',
    );
    // Cancelling the swipe settles the chat back over the list.
    await swipe.moveBy(const Offset(-160, 0));
    await swipe.up();
    await tester.pumpAndSettle();
    expect(controller.selectedId, 'chat');
    expect(list, findsNothing);

    await tester.tap(find.byType(BackButton));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(list, findsOneWidget, reason: 'the list stays beneath a pop');
    expect(placeholder, findsNothing);
    await tester.pumpAndSettle();
    expect(controller.selectedId, isNull);
    await tester.tap(list);
    expect(listTaps, 1);
    debugDefaultTargetPlatformOverride = null;
  });

  group('compact keyboard', () {
    Future<void> raiseKeyboard(WidgetTester tester) async {
      for (var frame = 1; frame <= 20; frame++) {
        tester.view.viewInsets = FakeViewPadding(bottom: frame * 15.0);
        await tester.pump(const Duration(milliseconds: 16));
      }
    }

    testWidgets('does not rebuild the covered list or detail routes', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(402, 874);
      addTearDown(tester.view.reset);
      final controller = ConversationPaneController();
      addTearDown(controller.dispose);
      var listBuilds = 0;
      var channelBuilds = 0;
      var threadBuilds = 0;
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: AdaptiveConversationPane(
              controller: controller,
              list: _CountedInsets('List', () => listBuilds++),
            ),
          ),
        ),
      );
      controller.open(
        'c',
        (_) => _CountedSize('Channel', () => channelBuilds++),
      );
      await tester.pumpAndSettle();
      listBuilds = 0;
      channelBuilds = 0;

      await raiseKeyboard(tester);

      expect(listBuilds, 0, reason: 'covered list');
      expect(channelBuilds, 0, reason: 'channel detail');

      Navigator.of(tester.element(find.text('Channel'))).push(
        MaterialPageRoute<void>(
          builder: (_) => _CountedSize('Thread', () => threadBuilds++),
        ),
      );
      await tester.pumpAndSettle();
      tester.view.viewInsets = FakeViewPadding.zero;
      await tester.pump();
      channelBuilds = 0;
      threadBuilds = 0;

      await raiseKeyboard(tester);

      expect(threadBuilds, 0, reason: 'thread on top');
      expect(channelBuilds, 0, reason: 'covered channel');
      debugDefaultTargetPlatformOverride = null;
    });

    testWidgets('list keeps its insets while a popped detail keyboard lowers', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(402, 874);
      addTearDown(tester.view.reset);
      final controller = ConversationPaneController();
      addTearDown(controller.dispose);
      final listInsets = <double>[];
      await tester.pumpWidget(
        ProviderScope(
          child: MaterialApp(
            home: AdaptiveConversationPane(
              controller: controller,
              list: Builder(
                builder: (context) {
                  listInsets.add(MediaQuery.viewInsetsOf(context).bottom);
                  return const Text('List');
                },
              ),
            ),
          ),
        ),
      );
      controller.open(
        'c',
        (_) => const Material(
          child: Column(
            children: [Text('Channel'), TextField(autofocus: true)],
          ),
        ),
      );
      await tester.pumpAndSettle();
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      await tester.pump();
      listInsets.clear();

      Navigator.of(tester.element(find.text('Channel'))).pop();
      await tester.pumpAndSettle();
      for (var bottom = 280.0; bottom >= 0; bottom -= 40) {
        tester.view.viewInsets = FakeViewPadding(bottom: bottom);
        await tester.pump(const Duration(milliseconds: 16));
      }

      expect(listInsets.where((bottom) => bottom != 0), isEmpty);
      debugDefaultTargetPlatformOverride = null;
    });
  });
}

class _Conversation extends StatelessWidget {
  const _Conversation();

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: const Text('Conversation')),
    body: Column(
      children: [
        Text('Visible: ${ConversationPaneScope.isActiveOf(context)}'),
        const TextField(),
        TextButton(
          onPressed: () => Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => Scaffold(
                appBar: AppBar(),
                body: const Text('Thread content'),
              ),
            ),
          ),
          child: const Text('Thread'),
        ),
      ],
    ),
  );
}

class _DeepLink extends StatefulWidget {
  const _DeepLink();
  @override
  State<_DeepLink> createState() => _DeepLinkState();
}

class _DeepLinkState extends State<_DeepLink> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => Scaffold(
            appBar: AppBar(),
            body: const Text('Deep-linked thread'),
          ),
        ),
      );
    });
  }

  @override
  Widget build(BuildContext context) =>
      const Scaffold(body: Text('Selected channel'));
}

class _CountedInsets extends StatelessWidget {
  const _CountedInsets(this.label, this.onBuild);
  final String label;
  final VoidCallback onBuild;
  @override
  Widget build(BuildContext context) {
    onBuild();
    MediaQuery.viewInsetsOf(context);
    return Text(label);
  }
}

class _CountedSize extends StatelessWidget {
  const _CountedSize(this.label, this.onBuild);
  final String label;
  final VoidCallback onBuild;
  @override
  Widget build(BuildContext context) {
    onBuild();
    MediaQuery.sizeOf(context);
    return Text(label);
  }
}
