import 'dart:convert';

import 'package:buzz/features/agents/agent_avatar_backdrop.dart';
import 'package:buzz/features/agents/agent_avatar_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final png = base64Decode(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
  );
  final first = AgentAvatarSelection.emoji(png, '😊', 0xFFFFCC00);
  final second = AgentAvatarSelection.emoji(png, '😊', 0xFF0090FF);

  Widget host(
    AgentAvatarSelection selection, {
    bool reduced = false,
    bool contrast = false,
  }) => MaterialApp(
    home: MediaQuery(
      data: MediaQueryData(disableAnimations: reduced, highContrast: contrast),
      child: SizedBox(
        width: 390,
        height: 800,
        child: AgentAvatarBackdrop(selection: selection),
      ),
    ),
  );

  testWidgets('rapid selection changes settle on the latest artwork tint', (
    tester,
  ) async {
    await tester.pumpWidget(host(first));
    await tester.pumpWidget(host(second));
    await tester.pump(const Duration(milliseconds: 80));
    expect(
      find.descendant(
        of: find.byType(AgentAvatarBackdrop),
        matching: find.byType(ColoredBox),
      ),
      findsNWidgets(2),
    );
    await tester.pumpWidget(host(first));
    await tester.pumpAndSettle();
    final colors = tester
        .widgetList<ColoredBox>(
          find.descendant(
            of: find.byType(AgentAvatarBackdrop),
            matching: find.byType(ColoredBox),
          ),
        )
        .map((box) => box.color);
    expect(colors, [Color(first.color!)]);
    expect(tester.takeException(), isNull);
  });

  testWidgets('reduced motion replaces the tint without a crossfade', (
    tester,
  ) async {
    await tester.pumpWidget(host(first, reduced: true));
    await tester.pumpWidget(host(second, reduced: true));
    await tester.pump();
    expect(tester.binding.transientCallbackCount, 0);
    final colors = tester
        .widgetList<ColoredBox>(
          find.descendant(
            of: find.byType(AgentAvatarBackdrop),
            matching: find.byType(ColoredBox),
          ),
        )
        .map((box) => box.color);
    expect(colors, [Color(second.color!)]);
  });

  testWidgets(
    'decorative artwork cannot intercept taps or add image semantics',
    (tester) async {
      final semantics = tester.ensureSemantics();
      var taps = 0;
      await tester.pumpWidget(
        MaterialApp(
          home: Stack(
            children: [
              Positioned.fill(
                child: GestureDetector(
                  onTap: () => taps++,
                  child: const ColoredBox(color: Colors.white),
                ),
              ),
              Positioned.fill(
                child: AgentAvatarBackdrop(
                  selection: AgentAvatarSelection.image(png),
                ),
              ),
            ],
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tapAt(const Offset(100, 100));
      expect(taps, 1);
      expect(find.bySemanticsLabel('Selected agent avatar'), findsNothing);
      semantics.dispose();
      await tester.pumpWidget(host(first, contrast: true));
      expect(find.byType(ImageFiltered), findsNothing);
    },
  );
}
