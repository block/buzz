import 'package:buzz/shared/widgets/bottom_up_page_route.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_action.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (_) async => null,
    );
  });
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      null,
    );
  });

  testWidgets('covered glass controls release their native channels', (
    tester,
  ) async {
    final messenger = binding.defaultBinaryMessenger;
    final navigator = GlobalKey<NavigatorState>();
    final label = ValueNotifier('Original');
    addTearDown(label.dispose);
    final dispatcher = tester.binding.platformDispatcher;
    dispatcher.platformBrightnessTestValue = Brightness.light;
    addTearDown(dispatcher.clearPlatformBrightnessTestValue);
    final channels = <String, MethodChannel>{};
    final disposed = <int>[];
    final calls = <({MethodChannel channel, MethodCall call})>[];
    final pressed = <String>[];
    addTearDown(() {
      for (final channel in channels.values) {
        messenger.setMockMethodCallHandler(channel, null);
      }
    });
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'create') {
        final args = call.arguments as Map;
        expect(args['viewType'], IosGlassNavigationButton.viewType);
        final params =
            const StandardMessageCodec().decodeMessage(
                  ByteData.sublistView(args['params'] as Uint8List),
                )
                as Map;
        final channel = MethodChannel('${args['viewType']}/${args['id']}');
        channels[params['accessibilityLabel'] as String] = channel;
        messenger.setMockMethodCallHandler(channel, (call) async {
          calls.add((channel: channel, call: call));
          return null;
        });
      } else if (call.method == 'dispose') {
        disposed.add(call.arguments as int);
      }
      return null;
    });

    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          navigatorKey: navigator,
          theme: ThemeData.light(),
          darkTheme: ThemeData.dark(),
          themeAnimationDuration: Duration.zero,
          home: Scaffold(
            body: ValueListenableBuilder<String>(
              valueListenable: label,
              builder: (context, value, _) => Row(
                children: [
                  IosGlassNavigationButton(
                    icon: IosGlassNavigationIcon.more,
                    semanticLabel: 'Button $value',
                    onPressed: () => pressed.add('button $value'),
                  ),
                  IosGlassNavigationAction(
                    label: 'Action $value',
                    onPressed: () => pressed.add('action $value'),
                  ),
                  IosMessageCreateButton(
                    accessibilityLabel: 'Create $value',
                    actions: [
                      (
                        label: value,
                        symbol: 'plus',
                        onPressed: () => pressed.add('menu $value'),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(channels, hasLength(3));
    final original = channels.values.toList();
    Future<void> pressControls(String label) async {
      for (final kind in ['Button', 'Action', 'Create']) {
        await messenger.handlePlatformMessage(
          channels['$kind $label']!.name,
          const StandardMethodCodec().encodeMethodCall(
            kind == 'Create'
                ? const MethodCall('menuAction', 0)
                : const MethodCall('pressed'),
          ),
          (_) {},
        );
      }
    }

    await pressControls('Original');
    expect(pressed, ['button Original', 'action Original', 'menu Original']);
    pressed.clear();
    navigator.currentState!.push(
      DialogRoute<void>(
        context: navigator.currentContext!,
        builder: (_) => const AlertDialog(content: Text('Covering dialog')),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byType(UiKitView), findsNothing);
    expect(
      disposed,
      unorderedEquals(
        original.map((channel) => int.parse(channel.name.split('/').last)),
      ),
    );
    calls.clear();
    dispatcher.platformBrightnessTestValue = Brightness.dark;
    label.value = 'Updated';
    await tester.pumpAndSettle();
    expect(calls, isEmpty, reason: 'covered controls must not send updates');
    await pressControls('Original');
    expect(pressed, isEmpty, reason: 'disposed controls have no handlers');

    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(channels, hasLength(6));
    final restored = channels.values.skip(3);
    expect(
      calls
          .where(
            (entry) =>
                entry.call.method == 'setAppearance' &&
                (entry.call.arguments as Map)['brightness'] == 'dark',
          )
          .map((entry) => entry.channel),
      unorderedEquals(restored),
    );
    expect(
      tester
          .widgetList<UiKitView>(find.byType(UiKitView))
          .map((view) => (view.creationParams as Map)['accessibilityLabel']),
      ['Button Updated', 'Action Updated', 'Create Updated'],
    );
    await pressControls('Updated');
    expect(pressed, ['button Updated', 'action Updated', 'menu Updated']);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  for (final bottomUp in [true, false]) {
    testWidgets(
      'glass controls respect covering route composition: bottomUp=$bottomUp',
      (tester) async {
        final navigator = GlobalKey<NavigatorState>();
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              navigatorKey: navigator,
              home: Scaffold(
                body: Row(
                  key: const ValueKey('underlying-controls'),
                  children: [
                    IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.more,
                      label: 'All',
                      semanticLabel: 'All',
                      onPressed: () {},
                    ),
                    IosGlassNavigationAction(label: 'Done', onPressed: () {}),
                    const IosMessageCreateButton(actions: []),
                  ],
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final controls = find.byKey(
          const ValueKey('underlying-controls'),
          skipOffstage: false,
        );
        final nativeViews = find.descendant(
          of: controls,
          matching: find.byType(UiKitView, skipOffstage: false),
        );
        final initialSize = tester.getSize(controls);
        final initialPosition = tester.getTopLeft(controls);
        expect(nativeViews, findsNWidgets(3));
        Widget destination(BuildContext context) => Scaffold(
          key: const ValueKey('moving-sheet'),
          body: IosGlassNavigationButton(
            icon: IosGlassNavigationIcon.close,
            semanticLabel: 'Close incoming screen',
            onPressed: () => Navigator.of(context).pop(),
          ),
        );
        navigator.currentState!.push(
          bottomUp
              ? BottomUpPageRoute<void>(
                  builder: destination,
                  reducedMotion: false,
                )
              : MaterialPageRoute<void>(builder: destination),
        );
        await tester.pump();
        expect(
          nativeViews,
          bottomUp ? findsNWidgets(3) : findsNothing,
          reason: 'sheets cover mounted controls; other routes suppress them',
        );
        expect(tester.getSize(controls), initialSize);
        await tester.pump(const Duration(milliseconds: 100));
        expect(nativeViews, bottomUp ? findsNWidgets(3) : findsNothing);
        expect(
          find.byWidgetPredicate(
            (w) => w is UiKitView && w.viewType == 'buzz/navigation_glass',
          ),
          bottomUp ? findsNWidgets(4) : findsOneWidget,
          reason: 'incoming control stays visible',
        );
        if (bottomUp) {
          expect(tester.getTopLeft(controls), initialPosition);
          final backing = find.byWidgetPredicate(
            (w) => w is UiKitView && w.viewType == 'buzz/toast_layer',
          );
          expect(backing, findsOneWidget);
          expect(
            tester.getSize(backing),
            tester.view.physicalSize / tester.view.devicePixelRatio,
          );
          final params =
              tester.widget<UiKitView>(backing).creationParams as Map;
          expect((params['color'] as int) >> 24, 255);
        }
        await tester.pumpAndSettle();
        navigator.currentState!.pop();
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 100));
        expect(
          nativeViews,
          bottomUp ? findsNWidgets(3) : findsNothing,
          reason: 'sheet reveals mounted controls throughout dismissal',
        );
        expect(
          find.descendant(
            of: find.byKey(const ValueKey('moving-sheet')),
            matching: find.byWidgetPredicate(
              (w) => w is UiKitView && w.viewType == 'buzz/navigation_glass',
            ),
          ),
          findsOneWidget,
          reason: 'the departing sheet keeps its own controls until it exits',
        );
        if (bottomUp) {
          expect(tester.getTopLeft(controls), initialPosition);
          expect(
            tester.getTopLeft(find.byKey(const ValueKey('moving-sheet'))).dy,
            greaterThan(0),
          );
        }
        await tester.pumpAndSettle();
        expect(nativeViews, findsNWidgets(3));
        expect(tester.getSize(controls), initialSize);
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }
}
