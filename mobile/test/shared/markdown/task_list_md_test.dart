import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';

import 'package:buzz/shared/markdown/task_list_md.dart';
import 'package:buzz/shared/markdown/task_markers.dart';

/// Renders [source] the way `message_content.dart` does — the custom task
/// component ahead of the stock block components, which is the ordering that
/// makes it win over `UnOrderedList`.
Future<void> pumpMarkdown(
  WidgetTester tester,
  String source, {
  TaskToggleCallback? onToggle,
}) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: SingleChildScrollView(
          child: GptMarkdown(
            source,
            components: taskAwareComponents(onToggle: onToggle),
          ),
        ),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  testWidgets('GFM task syntax renders as checkboxes, not as bullets', (
    tester,
  ) async {
    // The regression this whole component exists for: on the stock package
    // `UnOrderedList` claims `- [ ] ...` first and the text renders literally.
    await pumpMarkdown(tester, '- [ ] revisar el relay\n- [x] cerrar todo');

    expect(find.byType(Checkbox), findsNWidgets(2));
    expect(tester.widget<Checkbox>(find.byType(Checkbox).at(0)).value, isFalse);
    expect(tester.widget<Checkbox>(find.byType(Checkbox).at(1)).value, isTrue);
    // The marker itself must not survive into the visible text.
    expect(find.textContaining('[ ]'), findsNothing);
    expect(find.textContaining('[x]'), findsNothing);
  });

  testWidgets('ordinals follow document order', (tester) async {
    final taps = <List<Object>>[];
    await pumpMarkdown(
      tester,
      '- [ ] uno\n- [ ] dos\n- [ ] tres',
      onToggle: (index, checked) => taps.add([index, checked]),
    );

    await tester.tap(find.byType(Checkbox).at(2));
    await tester.pump();
    await tester.tap(find.byType(Checkbox).at(0));
    await tester.pump();

    expect(taps, [
      [2, true],
      [0, true],
    ]);
  });

  testWidgets('the ordinal agrees with countTaskMarkers on tricky input', (
    tester,
  ) async {
    // The invariant the feature rests on: if the renderer and the rewriter
    // disagree on which markers count, a tap flips the wrong line. Fenced
    // samples and a bracket with no trailing space are what break naive
    // counting.
    const source = '''
Pendientes:

```
- [ ] ejemplo en codigo
```

- [ ] revisar relay
- [ ]sin-espacio
- [x] cerrar incidente''';

    await pumpMarkdown(tester, source);

    expect(find.byType(Checkbox), findsNWidgets(countTaskMarkers(source)));
    expect(find.byType(Checkbox), findsNWidgets(2));
  });

  testWidgets('tapping ordinal N flips exactly the marker numbered N', (
    tester,
  ) async {
    const source = '- [ ] uno\n- [ ] dos\n- [ ] tres';
    int? seenIndex;
    bool? seenChecked;
    await pumpMarkdown(
      tester,
      source,
      onToggle: (index, checked) {
        seenIndex = index;
        seenChecked = checked;
      },
    );

    await tester.tap(find.byType(Checkbox).at(1));
    await tester.pump();

    final next = toggleTaskMarker(source, seenIndex!, seenChecked!);
    expect(next, '- [ ] uno\n- [x] dos\n- [ ] tres');
  });

  testWidgets('without a callback the checkbox is inert', (tester) async {
    await pumpMarkdown(tester, '- [ ] no editable');

    final box = tester.widget<Checkbox>(find.byType(Checkbox));
    expect(box.onChanged, isNull);
  });

  testWidgets('an ordered-list task also renders a checkbox', (tester) async {
    await pumpMarkdown(tester, '1. [ ] primero');
    expect(find.byType(Checkbox), findsOneWidget);
  });
}
