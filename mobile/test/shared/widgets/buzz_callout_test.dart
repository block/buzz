import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_callout.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final (width, scale) in [(320.0, 1.0), (320.0, 3.0), (1024.0, 1.0)]) {
    testWidgets(
      'stays on screen beside an edge target, $width wide at ${scale}x',
      (tester) async {
        tester.view.physicalSize = Size(width, 700);
        tester.view.devicePixelRatio = 1;
        tester.platformDispatcher.textScaleFactorTestValue = scale;
        addTearDown(tester.view.reset);
        addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.dark(),
            home: Scaffold(
              body: Align(
                alignment: Alignment.bottomRight,
                child: BuzzCallout(
                  message: 'A tip long enough to need room',
                  visible: true,
                  onDismiss: () {},
                  child: const SizedBox.square(
                    key: Key('target'),
                    dimension: 56,
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pump();
        final tip = tester.getRect(
          find.ancestor(
            of: find.text('A tip long enough to need room'),
            matching: find.byType(TapRegion),
          ),
        );
        final target = tester.getRect(find.byKey(const Key('target')));
        expect(tip.right, lessThanOrEqualTo(width - Grid.xs));
        expect(tip.left, greaterThanOrEqualTo(Grid.xs));
        expect(tip.bottom, lessThan(target.top));
      },
    );
  }
  cardTests();
}

void cardTests() {
  Future<Rect> pumpCard(
    WidgetTester tester, {
    required double width,
    double scale = 1,
    VoidCallback? onTap,
    VoidCallback? onDismiss,
    VoidCallback? onPage,
  }) async {
    tester.view.physicalSize = Size(width, 700);
    tester.view.devicePixelRatio = 1;
    tester.platformDispatcher.textScaleFactorTestValue = scale;
    addTearDown(tester.view.reset);
    addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Stack(
            children: [
              Center(
                child: TextButton(
                  onPressed: onPage ?? () {},
                  child: const Text('Page'),
                ),
              ),
              Align(
                alignment: Alignment.bottomRight,
                child: BuzzCallout.card(
                  message: 'Make one',
                  visible: true,
                  onTap: onTap ?? () {},
                  onDismiss: onDismiss ?? () {},
                  artwork: const [Text('Art')],
                  child: const SizedBox.square(
                    key: Key('target'),
                    dimension: 56,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
    await tester.pump();
    final card = tester.getRect(
      find
          .ancestor(of: find.text('Make one'), matching: find.byType(Stack))
          .first,
    );
    expect(
      card.bottom,
      lessThan(tester.getRect(find.byKey(const Key('target'))).top),
    );
    return card;
  }

  testWidgets('a card spans a phone between the margins', (tester) async {
    final card = await pumpCard(tester, width: 320);
    expect(card.left, Grid.xs);
    expect(card.right, 320 - Grid.xs);
  });

  testWidgets('a card grows to fit large text below its close button', (
    tester,
  ) async {
    final card = await pumpCard(tester, width: 320, scale: 3);
    final message = tester.getRect(find.text('Make one'));
    final close = tester.getRect(find.byTooltip('Dismiss'));
    expect(card.contains(message.topLeft), isTrue);
    expect(card.contains(message.bottomRight), isTrue);
    expect(message.top, greaterThanOrEqualTo(close.bottom));
  });

  testWidgets('a card stays phone-sized on a wide screen', (tester) async {
    final card = await pumpCard(tester, width: 1024);
    expect(card.width, Grid.xxxl * 6);
    expect(card.right, lessThanOrEqualTo(1024 - Grid.xs));
  });

  testWidgets('a card opens on tap and closes only from its close button', (
    tester,
  ) async {
    var taps = 0;
    var dismissals = 0;
    var pageTaps = 0;
    await pumpCard(
      tester,
      width: 390,
      onTap: () => taps++,
      onDismiss: () => dismissals++,
      onPage: () => pageTaps++,
    );
    expect(find.text('Art'), findsOneWidget);
    final semantics = tester.ensureSemantics();
    // The close button stays its own control inside the tappable card.
    expect(
      tester.getSemantics(find.byTooltip('Dismiss')),
      matchesSemantics(
        tooltip: 'Dismiss',
        isButton: true,
        hasTapAction: true,
        hasEnabledState: true,
        isEnabled: true,
        isFocusable: true,
        hasFocusAction: true,
      ),
    );
    semantics.dispose();
    await tester.tap(find.text('Page'));
    expect((pageTaps, taps, dismissals), (1, 0, 0));
    await tester.tap(find.text('Make one'));
    expect((pageTaps, taps, dismissals), (1, 1, 0));
    await tester.tap(find.byTooltip('Dismiss'));
    expect((pageTaps, taps, dismissals), (1, 1, 1));
  });
}
