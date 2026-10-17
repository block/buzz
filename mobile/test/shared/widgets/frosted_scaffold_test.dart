import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:buzz/shared/widgets/ios_navigation_bar.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Border? _appBarBorder(WidgetTester tester) {
  final container = tester.widget<Container>(
    find.byKey(const ValueKey('frosted-app-bar-background')),
  );
  return (container.decoration as BoxDecoration?)?.border as Border?;
}

void main() {
  testWidgets('unbounded reversed scroll depth stays finite for native bar', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: FrostedScaffold(
          appBar: FrostedAppBar(title: Text('Conversation')),
          body: SizedBox(key: ValueKey('timeline')),
        ),
      ),
    );
    final source = tester.element(find.byKey(const ValueKey('timeline')));
    final offset = IosNavigationScrollScope.maybeOf(source)!;
    ScrollUpdateNotification(
      metrics: FixedScrollMetrics(
        minScrollExtent: 0,
        maxScrollExtent: double.infinity,
        pixels: 0,
        viewportDimension: 600,
        axisDirection: AxisDirection.up,
        devicePixelRatio: 1,
      ),
      context: source,
      scrollDelta: 0,
    ).dispatch(source);
    await tester.pump();
    expect(offset.value, 600);
  });

  testWidgets('foreground paints above header and leaves body interactive', (
    tester,
  ) async {
    var taps = 0;
    const marker = Text('Foreground date');
    const bar = FrostedAppBar(title: Text('Header'));
    await tester.pumpWidget(
      MaterialApp(
        home: FrostedScaffold(
          appBar: bar,
          body: Center(
            child: TextButton(
              onPressed: () => taps++,
              child: const Text('Body action'),
            ),
          ),
          foreground: const Stack(
            children: [Positioned(top: 80, left: 0, child: marker)],
          ),
        ),
      ),
    );
    final stack = tester.widget<Stack>(
      find.ancestor(of: find.byWidget(bar), matching: find.byType(Stack)).first,
    );
    expect(stack.children.indexOf(bar), lessThan(stack.children.length - 1));
    expect(
      find.descendant(
        of: find.byWidget(stack.children.last),
        matching: find.byWidget(marker),
      ),
      findsOneWidget,
    );
    await tester.tap(find.text('Body action'));
    expect(taps, 1);
  });

  testWidgets('utility pages expose the inverted surface hierarchy', (
    tester,
  ) async {
    const pageSurface = Color(0xFFEEEEEE);
    const containerSurface = Color(0xFFFFFFFF);

    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(
          colorScheme: lightColorScheme.copyWith(
            surface: containerSurface,
            surfaceContainerHighest: pageSurface,
          ),
        ),
        home: FrostedScaffold(
          useUtilitySurfaceTheme: true,
          appBar: const FrostedAppBar(title: Text('Settings')),
          body: Builder(
            builder: (context) => Column(
              children: [
                ColoredBox(
                  key: const ValueKey('utility-page-surface'),
                  color: Theme.of(context).colorScheme.surface,
                  child: const SizedBox.square(dimension: 20),
                ),
                ColoredBox(
                  key: const ValueKey('utility-container-surface'),
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                  child: const SizedBox.square(dimension: 20),
                ),
              ],
            ),
          ),
        ),
      ),
    );

    expect(
      tester
          .widget<ColoredBox>(
            find.byKey(const ValueKey('utility-page-surface')),
          )
          .color,
      pageSurface,
    );
    expect(
      tester
          .widget<ColoredBox>(
            find.byKey(const ValueKey('utility-container-surface')),
          )
          .color,
      containerSurface,
    );
  });

  for (final reverse in [false, true]) {
    testWidgets(
      'composer scrolling preserves timeline frost reverse=$reverse',
      (tester) async {
        tester.view.physicalSize = const Size(400, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final timeline = ScrollController(
          initialScrollOffset: reverse ? 0 : 120,
        );
        addTearDown(timeline.dispose);
        final composer = TextEditingController();
        addTearDown(composer.dispose);
        await tester.pumpWidget(
          MaterialApp(
            theme: AppTheme.light(),
            home: FrostedScaffold(
              appBar: const FrostedAppBar(title: Text('Conversation')),
              body: Column(
                children: [
                  Expanded(
                    child: ListView.builder(
                      reverse: reverse,
                      controller: timeline,
                      itemCount: 40,
                      itemBuilder: (_, index) =>
                          SizedBox(height: 48, child: Text('Message $index')),
                    ),
                  ),
                  SizedBox(
                    height: 100,
                    child: TextField(
                      controller: composer,
                      minLines: 1,
                      maxLines: 3,
                    ),
                  ),
                ],
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        expect(find.byType(BackdropFilter), findsOneWidget);
        await tester.tap(find.byType(TextField));
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.enterText(
          find.byType(TextField),
          'One\nTwo\nThree\nFour\nFive',
        );
        await tester.pumpAndSettle();
        expect(find.byType(BackdropFilter), findsOneWidget);
        await tester.enterText(find.byType(TextField), '');
        await tester.pumpAndSettle();
        expect(find.byType(BackdropFilter), findsOneWidget);
        expect(_appBarBorder(tester)?.bottom.color.a, greaterThan(0));
        timeline.jumpTo(reverse ? timeline.position.maxScrollExtent : 0);
        await tester.pumpAndSettle();
        expect(find.byType(BackdropFilter), findsNothing);
        await tester.enterText(
          find.byType(TextField),
          'One\nTwo\nThree\nFour\nFive',
        );
        await tester.pumpAndSettle();
        expect(find.byType(BackdropFilter), findsNothing);
      },
    );
  }

  testWidgets(
    'page frost and divider appear only while content scrolls under header',
    (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: FrostedScaffold(
            appBar: const FrostedAppBar(title: Text('Theme')),
            body: ListView.builder(
              key: const ValueKey('page-scroll-view'),
              padding: const EdgeInsets.only(top: 57),
              itemCount: 40,
              itemBuilder: (_, index) =>
                  SizedBox(height: 48, child: Text('Theme option $index')),
            ),
          ),
        ),
      );

      expect(_appBarBorder(tester)?.bottom.color.a, 0);
      expect(find.byType(BackdropFilter), findsNothing);

      await tester.drag(
        find.byKey(const ValueKey('page-scroll-view')),
        const Offset(0, -120),
      );
      await tester.pumpAndSettle();

      final border = _appBarBorder(tester);
      expect(border, isNotNull);
      expect(border!.bottom.color.a, greaterThan(0));
      expect(find.byType(BackdropFilter), findsOneWidget);

      await tester.drag(
        find.byKey(const ValueKey('page-scroll-view')),
        const Offset(0, 500),
      );
      await tester.pumpAndSettle();

      expect(_appBarBorder(tester)?.bottom.color.a, 0);
      expect(find.byType(BackdropFilter), findsNothing);
    },
  );
}
