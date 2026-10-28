import 'package:buzz/features/agents/agents_sheet.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'agents_harness.dart';

void main() {
  final existing = 'a' * 64;

  testWidgets('agent detail pushes inside one modal and Back restores list', (
    tester,
  ) async {
    final h = await AgentsHarness.create();
    await h.open(tester, directory: [existing]);
    final list = tester.element(find.byType(AgentsSheet));
    await tester.tap(find.byKey(ValueKey(existing)));
    await tester.pump();
    for (var frame = 0; frame < 3; frame++) {
      await tester.pump(const Duration(milliseconds: 60));
      final surface = find.byKey(const ValueKey('agent-detail-surface'));
      expect(tester.widget<Material>(surface).color?.a, 1);
      expect(tester.takeException(), isNull);
    }
    await tester.pumpAndSettle();
    expect(find.byType(BottomSheet), findsOneWidget);
    expect(find.byType(UserProfileSheet), findsOneWidget);
    expect(list.mounted, isTrue);
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsNothing);
    expect(find.byKey(ValueKey(existing)), findsOneWidget);
    expect(tester.element(find.byType(AgentsSheet)), same(list));
    await tester.tap(find.byTooltip('Close'));
    await tester.pumpAndSettle();
    expect(find.byType(BottomSheet), findsNothing);
  });
}
