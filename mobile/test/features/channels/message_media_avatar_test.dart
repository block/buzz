import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_media_avatar.dart';

void main() {
  testWidgets('caption edits retain offset but replacing images resets it', (
    tester,
  ) async {
    late ValueNotifier<double> offset;
    Widget build(String caption, String second) => MaterialApp(
      home: HookBuilder(
        builder: (context) {
          offset = useMessageMediaOffset(
            '$caption\n![one](https://example.com/one.png)\n![two](https://example.com/$second.png)',
            const [],
          );
          return MessageMediaAvatar(
            offset: offset,
            child: const SizedBox(width: 42, height: 42),
          );
        },
      ),
    );
    await tester.pumpWidget(build('Original', 'two'));
    offset.value = 200;
    await tester.pump();
    await tester.pumpWidget(build('Edited caption', 'two'));
    expect(offset.value, 200);
    await tester.pumpWidget(build('Edited caption', 'three'));
    expect(offset.value, 0);
  });

  testWidgets('gallery displacement hides the complete profile action', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();

    final offset = ValueNotifier<double>(0);
    addTearDown(offset.dispose);
    var taps = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: MessageMediaAvatar(
          offset: offset,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => taps++,
            child: Semantics(
              label: 'Open Alice profile',
              child: const SizedBox(width: 40, height: 40),
            ),
          ),
        ),
      ),
    );
    for (final displacement in [0.0, -100.0, 100.0, 0.0]) {
      offset.value = displacement;
      await tester.pump();
      final hidden = displacement > 0;
      var hasProfileAction = false;
      void visit(SemanticsNode node) {
        if (node.label == 'Open Alice profile' &&
            node.getSemanticsData().hasAction(SemanticsAction.tap)) {
          hasProfileAction = true;
        }
        node.visitChildren((child) {
          visit(child);
          return true;
        });
      }

      visit(
        tester
            .binding
            .renderViews
            .single
            .owner!
            .semanticsOwner!
            .rootSemanticsNode!,
      );
      expect(hasProfileAction, !hidden);
      final opacity = tester.widget<Opacity>(
        find.descendant(
          of: find.byType(MessageMediaAvatar),
          matching: find.byType(Opacity),
        ),
      );
      expect(opacity.opacity, hidden ? 0 : 1);
    }
    await tester.tap(find.byType(MessageMediaAvatar));
    expect(taps, 1);
    semantics.dispose();
  });
}
