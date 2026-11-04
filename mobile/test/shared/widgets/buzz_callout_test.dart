import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_callout.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  Future<Rect> pumpCard(
    WidgetTester tester, {
    required double width,
    double height = 700,
    String message = 'Make one',
    double scale = 1,
    VoidCallback? onTap,
    VoidCallback? onDismiss,
    VoidCallback? onPage,
  }) async {
    tester.view.physicalSize = Size(width, height);
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
                child: BuzzCallout(
                  message: message,
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
    await tester.pumpAndSettle();
    final card = tester.getRect(
      find.ancestor(of: find.text(message), matching: find.byType(Stack)).first,
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

  testWidgets('a card scrolls large text that outgrows a short screen', (
    tester,
  ) async {
    const message = 'Create your first agent';
    final card = await pumpCard(
      tester,
      width: 568,
      height: 320,
      message: message,
      scale: 3,
    );
    expect(tester.takeException(), isNull);
    final close = tester.getRect(find.byTooltip('Dismiss'));
    expect(card.contains(close.topLeft), isTrue);
    expect(card.contains(close.bottomRight), isTrue);
    final scrollable = find.descendant(
      of: find.byType(BuzzCallout),
      matching: find.byType(Scrollable),
    );
    expect(
      tester.state<ScrollableState>(scrollable).position.maxScrollExtent,
      greaterThan(0),
    );
    await tester.drag(scrollable, const Offset(0, -1000));
    await tester.pumpAndSettle();
    expect(
      tester.getRect(find.text(message)).bottom,
      lessThanOrEqualTo(card.bottom),
    );
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
