import 'dart:async';

import 'package:buzz/shared/widgets/bee_refresh_indicator.dart';
import 'package:buzz/shared/widgets/flapping_bee.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('shows the bee while pulling to refresh', (tester) async {
    const contentKey = ValueKey('loading-content');
    var refreshes = 0;
    final refreshCompleter = Completer<void>();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: BeeRefreshIndicator(
          onRefresh: () {
            refreshes++;
            return refreshCompleter.future;
          },
          child: ListView(
            children: const [SizedBox(key: contentKey, height: 800)],
          ),
        ),
      ),
    );

    final listFinder = find.byType(ListView);
    final restingTop = tester.getTopLeft(listFinder).dy;
    final restingContentTop = tester.getTopLeft(find.byKey(contentKey)).dy;
    await tester.timedDrag(
      listFinder,
      const Offset(0, 320),
      const Duration(milliseconds: 500),
    );
    await tester.pump(const Duration(milliseconds: 16));
    await tester.pump(const Duration(milliseconds: 300));

    final beeFinder = find.byType(FlappingBee);
    final loadingTop = tester.getTopLeft(listFinder).dy;
    final loadingContentTop = tester.getTopLeft(find.byKey(contentKey)).dy;
    final gapTransform = tester.widget<Transform>(
      find.byKey(const ValueKey('bee-refresh-retained-gap')),
    );
    expect(beeFinder, findsOneWidget);
    expect(refreshes, 1);
    expect(gapTransform.transform.getTranslation().y, closeTo(72, 1));
    expect(loadingTop - restingTop, closeTo(72, 1));
    final loadingBeeRect = tester.getRect(beeFinder);
    final loadingGap = loadingContentTop - restingContentTop;
    expect(
      loadingBeeRect.center.dy,
      closeTo(
        restingContentTop +
            (loadingGap - loadingBeeRect.height) * 0.75 +
            loadingBeeRect.height / 2,
        1,
      ),
    );

    refreshCompleter.complete();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 90));

    final closingTop = tester.getTopLeft(listFinder).dy;
    expect(closingTop, greaterThan(restingTop));
    expect(closingTop, lessThan(loadingTop));

    await tester.pumpAndSettle();
    expect(tester.getTopLeft(listFinder).dy, closeTo(restingTop, 1));
    expect(beeFinder, findsNothing);
  });

  testWidgets('tracks each stage of an active pull', (tester) async {
    tester.view.physicalSize = const Size(420, 912);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') hapticCalls.add(call);
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    const contentKey = ValueKey('pull-content');
    final refreshCompleter = Completer<void>();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: BeeRefreshIndicator(
          onRefresh: () => refreshCompleter.future,
          child: ListView(
            children: const [SizedBox(key: contentKey, height: 800)],
          ),
        ),
      ),
    );

    final restingContentTop = tester.getTopLeft(find.byKey(contentKey)).dy;
    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(ListView)),
      pointer: 1,
    );
    await gesture.moveBy(const Offset(0, 12));
    await tester.pump();

    final beeFinder = find.byType(FlappingBee);
    expect(beeFinder, findsNothing);
    expect(
      tester.getTopLeft(find.byKey(contentKey)).dy,
      greaterThan(restingContentTop),
    );

    await gesture.moveBy(const Offset(0, 44));
    await tester.pump();

    final earlyTop = tester.getTopLeft(beeFinder).dy;
    final partialOpacity = tester.widget<Opacity>(
      find.byKey(const ValueKey('bee-refresh-opacity')),
    );
    expect(partialOpacity.opacity, greaterThan(0));
    expect(partialOpacity.opacity, lessThan(1));
    final partialScale = tester
        .widget<Transform>(find.byKey(const ValueKey('bee-refresh-scale')))
        .transform
        .storage[0];
    expect(partialScale, greaterThan(0.6));
    expect(partialScale, lessThan(1));
    expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
    expect(hapticCalls, isEmpty);

    await gesture.moveBy(const Offset(0, 120));
    await tester.pump();

    final pulledContentTop = tester.getTopLeft(find.byKey(contentKey)).dy;
    expect(tester.getTopLeft(beeFinder).dy, greaterThan(earlyTop));
    expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
    expect(find.byKey(const ValueKey('bee-refresh-eyes-emoji')), findsNothing);
    final pulledBeeRect = tester.getRect(beeFinder);
    final pulledGap = pulledContentTop - restingContentTop;
    expect(
      pulledBeeRect.center.dy,
      closeTo(
        restingContentTop +
            (pulledGap - pulledBeeRect.height) * 0.75 +
            pulledBeeRect.height / 2,
        1,
      ),
    );

    await gesture.moveBy(const Offset(0, 120));
    await tester.pump();

    expect(
      tester
          .widget<Transform>(find.byKey(const ValueKey('bee-refresh-scale')))
          .transform
          .storage[0],
      closeTo(1, 0.001),
    );
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.mediumImpact');
    expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
    expect(find.byKey(const ValueKey('bee-refresh-eyes-emoji')), findsNothing);

    await tester.pump(const Duration(milliseconds: 350));
    await gesture.moveBy(
      const Offset(0, 120),
      timeStamp: const Duration(milliseconds: 350),
    );
    await tester.pump();

    final pupilProgress = tester.widget<FlappingBee>(beeFinder).eyeProgress;
    expect(pupilProgress, greaterThan(0));
    expect(pupilProgress, lessThan(0.5));
    expect(find.byKey(const ValueKey('bee-refresh-eyes-emoji')), findsNothing);
    expect(hapticCalls, hasLength(1));

    await tester.pump(const Duration(milliseconds: 400));
    await gesture.moveBy(
      const Offset(0, 240),
      timeStamp: const Duration(milliseconds: 750),
    );
    await tester.pump();

    expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
    expect(
      find.byKey(const ValueKey('bee-refresh-eyes-emoji')),
      findsOneWidget,
    );
    expect(hapticCalls, hasLength(2));
    expect(hapticCalls.last.arguments, 'HapticFeedbackType.heavyImpact');
    expect(
      tester
          .widget<Transform>(
            find.byKey(const ValueKey('bee-refresh-eyes-emoji-offset')),
          )
          .transform
          .storage[12],
      2,
    );
    final initialShake = tester
        .widget<Transform>(
          find.byKey(const ValueKey('bee-refresh-eyes-emoji-shake')),
        )
        .transform
        .storage[12];
    await tester.pump(const Duration(milliseconds: 35));
    final movedShake = tester
        .widget<Transform>(
          find.byKey(const ValueKey('bee-refresh-eyes-emoji-shake')),
        )
        .transform
        .storage[12];
    expect(movedShake, isNot(closeTo(initialShake, 0.01)));
    expect(movedShake.abs(), lessThanOrEqualTo(0.75));
    await tester.pump(const Duration(milliseconds: 105));
    expect(hapticCalls, hasLength(3));
    expect(hapticCalls.last.arguments, 'HapticFeedbackType.selectionClick');

    final secondGesture = await tester.startGesture(
      tester.getCenter(find.byType(ListView)),
      pointer: 2,
    );
    await secondGesture.moveBy(
      const Offset(0, 40),
      timeStamp: const Duration(milliseconds: 800),
    );
    await gesture.up();
    await tester.pump();
    expect(
      find.byKey(const ValueKey('bee-refresh-eyes-emoji')),
      findsOneWidget,
    );
    expect(hapticCalls, hasLength(3));

    await secondGesture.moveBy(
      const Offset(0, 80),
      timeStamp: const Duration(milliseconds: 900),
    );
    await tester.pump();
    expect(
      find.byKey(const ValueKey('bee-refresh-eyes-emoji')),
      findsOneWidget,
    );
    expect(hapticCalls, hasLength(3));

    await secondGesture.up();
    await tester.pump();
    final hapticsAfterRelease = hapticCalls.length;
    await tester.pump(const Duration(milliseconds: 300));

    expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
    expect(find.byKey(const ValueKey('bee-refresh-eyes-emoji')), findsNothing);
    expect(hapticCalls, hasLength(hapticsAfterRelease));

    refreshCompleter.complete();
    await tester.pumpAndSettle();
  });

  testWidgets('keeps expressive eyes hidden for a quick refresh flick', (
    tester,
  ) async {
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') hapticCalls.add(call);
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );
    final refreshCompleter = Completer<void>();
    var refreshes = 0;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: BeeRefreshIndicator(
          onRefresh: () {
            refreshes++;
            return refreshCompleter.future;
          },
          child: ListView(children: const [SizedBox(height: 800)]),
        ),
      ),
    );

    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(ListView)),
    );
    final beeFinder = find.byType(FlappingBee);
    for (var step = 0; step < 10; step++) {
      await gesture.moveBy(
        const Offset(0, 50),
        timeStamp: Duration(milliseconds: (step + 1) * 10),
      );
      await tester.pump(const Duration(milliseconds: 10));
      if (beeFinder.evaluate().isNotEmpty) {
        expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
        expect(
          find.byKey(const ValueKey('bee-refresh-eyes-emoji')),
          findsNothing,
        );
      }
    }

    await gesture.up();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));

    expect(refreshes, 1);
    expect(hapticCalls, hasLength(1));
    expect(hapticCalls.single.arguments, 'HapticFeedbackType.mediumImpact');
    expect(tester.widget<FlappingBee>(beeFinder).eyeProgress, isNull);
    expect(find.byKey(const ValueKey('bee-refresh-eyes-emoji')), findsNothing);

    refreshCompleter.complete();
    await tester.pumpAndSettle();
  });

  testWidgets('keeps the bee static when motion is disabled', (tester) async {
    await tester.pumpWidget(
      MediaQuery(
        data: const MediaQueryData(disableAnimations: true),
        child: WidgetHelpers.testable(
          child: Builder(
            builder: (context) => MediaQuery(
              data: MediaQuery.of(context).copyWith(disableAnimations: true),
              child: BeeRefreshIndicator(
                onRefresh: () async {},
                child: ListView(children: const [SizedBox(height: 800)]),
              ),
            ),
          ),
        ),
      ),
    );

    await tester.timedDrag(
      find.byType(ListView),
      const Offset(0, 160),
      const Duration(milliseconds: 400),
    );
    await tester.pump();

    final bee = tester.widget<FlappingBee>(find.byType(FlappingBee));
    expect(bee.flapAmount, 0);
  });

  testWidgets('provides elastic always-scrollable physics', (tester) async {
    late ScrollPhysics physics;

    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: BeeRefreshIndicator(
          onRefresh: () async {},
          child: Builder(
            builder: (context) {
              physics = ScrollConfiguration.of(
                context,
              ).getScrollPhysics(context);
              return ListView(children: const [SizedBox(height: 20)]);
            },
          ),
        ),
      ),
    );

    expect(physics, isA<BouncingScrollPhysics>());
    expect(physics.parent, isA<AlwaysScrollableScrollPhysics>());
  });
}
