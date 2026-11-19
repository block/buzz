import 'dart:async';

import 'package:buzz/shared/widgets/bottom_up_page_route.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_action.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:buzz/shared/widgets/ios_message_create_button.dart';
import 'package:buzz/shared/widgets/native_route_controls.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
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

  testWidgets(
    'covered glass controls stay mounted; create control releases its channel',
    (tester) async {
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
      final button = channels['Button Original']!;
      final action = channels['Action Original']!;
      final create = channels['Create Original']!;
      navigator.currentState!.push(
        DialogRoute<void>(
          context: navigator.currentContext!,
          builder: (_) => const AlertDialog(content: Text('Covering dialog')),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(UiKitView), findsNWidgets(2));
      expect(disposed, [int.parse(create.name.split('/').last)]);
      calls.clear();
      dispatcher.platformBrightnessTestValue = Brightness.dark;
      label.value = 'Updated';
      await tester.pumpAndSettle();
      final darkUpdates = calls
          .where(
            (entry) =>
                entry.call.method == 'setAppearance' &&
                (entry.call.arguments as Map)['brightness'] == 'dark',
          )
          .map((entry) => entry.channel);
      expect(
        darkUpdates,
        unorderedEquals([button, action]),
        reason: 'covered glass controls stay mounted and keep updating',
      );
      expect(
        calls.where((entry) => entry.channel == create),
        isEmpty,
        reason: 'the unmounted create control must not receive updates',
      );

      navigator.currentState!.pop();
      await tester.pumpAndSettle();
      expect(channels, hasLength(4));
      expect(disposed, hasLength(1));
      expect(find.byType(UiKitView), findsNWidgets(3));
      for (final channel in [button, action]) {
        await messenger.handlePlatformMessage(
          channel.name,
          const StandardMethodCodec().encodeMethodCall(
            const MethodCall('pressed'),
          ),
          (_) {},
        );
      }
      await messenger.handlePlatformMessage(
        channels['Create Updated']!.name,
        const StandardMethodCodec().encodeMethodCall(
          const MethodCall('menuAction', 0),
        ),
        (_) {},
      );
      expect(pressed, ['button Updated', 'action Updated', 'menu Updated']);
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    },
  );

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
          bottomUp ? findsNWidgets(3) : findsNWidgets(2),
          reason:
              'sheets cover mounted controls; other routes suppress only '
              'the create control',
        );
        expect(tester.getSize(controls), initialSize);
        await tester.pump(const Duration(milliseconds: 100));
        expect(nativeViews, bottomUp ? findsNWidgets(3) : findsNWidgets(2));
        expect(
          find.byWidgetPredicate(
            (w) => w is UiKitView && w.viewType == 'buzz/navigation_glass',
          ),
          bottomUp ? findsNWidgets(4) : findsNWidgets(3),
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
          bottomUp ? findsNWidgets(3) : findsNWidgets(2),
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

  testWidgets('glass controls stay mounted under a modal bottom sheet', (
    tester,
  ) async {
    final creates = <int>[];
    final disposes = <int>[];
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform_views,
      (call) async {
        if (call.method == 'create') {
          creates.add((call.arguments as Map)['id'] as int);
        } else if (call.method == 'dispose') {
          disposes.add(call.arguments as int);
        }
        return null;
      },
    );
    final navigator = GlobalKey<NavigatorState>();
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          navigatorKey: navigator,
          home: Scaffold(
            body: Row(
              children: [
                IosGlassNavigationButton(
                  icon: IosGlassNavigationIcon.more,
                  semanticLabel: 'More',
                  onPressed: () {},
                ),
                IosGlassNavigationAction(label: 'Done', onPressed: () {}),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(creates, hasLength(2));
    unawaited(
      showModalBottomSheet<void>(
        context: navigator.currentContext!,
        builder: (_) => const SizedBox(height: 200, child: Text('Sheet')),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byType(UiKitView), findsNWidgets(2));
    navigator.currentState!.pop();
    await tester.pumpAndSettle();
    expect(find.byType(UiKitView), findsNWidgets(2));
    expect(creates, hasLength(2));
    expect(disposes, isEmpty);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('route visibility rebuilds only when it changes', (tester) async {
    final navigator = GlobalKey<NavigatorState>();
    final builds = <bool>[];
    await tester.pumpWidget(
      MaterialApp(
        navigatorKey: navigator,
        home: HookBuilder(
          builder: (context) {
            builds.add(useNativeRouteControlsVisible(context));
            return const SizedBox();
          },
        ),
      ),
    );
    await tester.pumpAndSettle();
    builds.clear();
    navigator.currentState!.push(
      MaterialPageRoute<void>(builder: (_) => const SizedBox()),
    );
    var frames = 0;
    await tester.pump();
    while (tester.binding.hasScheduledFrame) {
      await tester.pump(const Duration(milliseconds: 16));
      frames++;
    }
    navigator.currentState!.pop();
    await tester.pump();
    while (tester.binding.hasScheduledFrame) {
      await tester.pump(const Duration(milliseconds: 16));
      frames++;
    }
    expect(frames, greaterThan(20), reason: 'both transitions animated');
    expect(builds, isNotEmpty);
    expect(builds.last, isTrue);
    expect(
      builds.length,
      lessThanOrEqualTo(6),
      reason: 'visibility must not rebuild its host on every frame',
    );
    debugDefaultTargetPlatformOverride = null;
  });
}
