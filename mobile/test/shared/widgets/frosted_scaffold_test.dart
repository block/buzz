import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

Border? _appBarBorder(WidgetTester tester) {
  final container = tester.widget<Container>(
    find.byKey(const ValueKey('frosted-app-bar-background')),
  );
  return (container.decoration as BoxDecoration?)?.border as Border?;
}

void main() {
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

  testWidgets('page divider appears only after content scrolls under header', (
    tester,
  ) async {
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

    await tester.drag(
      find.byKey(const ValueKey('page-scroll-view')),
      const Offset(0, -120),
    );
    await tester.pumpAndSettle();

    final border = _appBarBorder(tester);
    expect(border, isNotNull);
    expect(border!.bottom.color.a, greaterThan(0));

    await tester.drag(
      find.byKey(const ValueKey('page-scroll-view')),
      const Offset(0, 500),
    );
    await tester.pumpAndSettle();

    expect(_appBarBorder(tester)?.bottom.color.a, 0);
  });
}
