import 'dart:ui' show SemanticsAction;

import 'package:buzz/features/agents/agent_card.dart';
import 'package:buzz/features/agents/agent_card_grid.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

void main() {
  testWidgets('whole card opens profile without stealing footer taps', (
    tester,
  ) async {
    var opens = 0;
    var selections = 0;
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: SizedBox(
                width: 180,
                height: 240,
                child: AgentCard(
                  name: 'Cloud assistant',
                  onOpen: () => opens++,
                  footer: TextButton(
                    onPressed: () => selections++,
                    child: const Text('Model'),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    final bounds = tester.getRect(find.byType(AgentCard));
    await tester.tapAt(Offset(bounds.left + 2, bounds.center.dy));
    await tester.tapAt(Offset(bounds.center.dx, bounds.bottom - 2));
    expect(opens, 2);
    await tester.tap(find.text('Model'));
    expect(selections, 1);
    expect(opens, 2);
    expect(tester.takeException(), isNull);
  });

  testWidgets('agent grid keeps cards readable and actionable at large text', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final semantics = tester.ensureSemantics();
    try {
      String? opened;

      Future<void> render(double scale) async {
        await tester.pumpWidget(
          ProviderScope(
            child: MaterialApp(
              theme: AppTheme.light(),
              home: MediaQuery(
                data: MediaQueryData(
                  size: const Size(390, 844),
                  textScaler: TextScaler.linear(scale),
                  disableAnimations: true,
                ),
                child: Scaffold(
                  body: CustomScrollView(
                    slivers: [
                      AgentCardGrid(
                        itemCount: 6,
                        itemBuilder: (_, index) => AgentCard(
                          name: 'Research assistant $index',
                          location: 'Remote',
                          onOpen: () => opened = '$index',
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
      }

      await render(1);
      expect(
        tester.getTopLeft(find.byType(AgentCard).at(0)).dy,
        tester.getTopLeft(find.byType(AgentCard).at(1)).dy,
      );
      await render(2);
      expect(
        tester.getTopLeft(find.byType(AgentCard).at(1)).dy,
        greaterThan(tester.getTopLeft(find.byType(AgentCard).at(0)).dy),
      );
      final first = find.bySemanticsLabel(RegExp('Open Research assistant 0'));
      expect(
        tester
            .getSemantics(first)
            .getSemanticsData()
            .hasAction(SemanticsAction.tap),
        isTrue,
      );
      await tester.tap(first);
      expect(opened, '0');
      expect(tester.takeException(), isNull);
    } finally {
      semantics.dispose();
    }
  });
}
