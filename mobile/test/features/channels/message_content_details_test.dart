import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _board = '''Fleet work status
:::details Fleet working (2)
- model routing pilot
- openclaw upgrade
:::
:::details Private channels (1)
- personal website
:::
Footer''';

Widget _app(ValueNotifier<String> content, {String? messageId = 'event-1'}) =>
    ProviderScope(
      overrides: [customEmojiListProvider.overrideWith((ref) => const [])],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: AppMarkdownTheme(
          child: Scaffold(
            body: ValueListenableBuilder<String>(
              valueListenable: content,
              builder: (_, value, _) =>
                  MessageContent(content: value, messageId: messageId),
            ),
          ),
        ),
      ),
    );

/// Taps the section toggle; the title text itself is inert by design.
Future<void> _tapTitle(WidgetTester tester, String title) => tester.tap(
  find
      .ancestor(
        of: find.textContaining(title, findRichText: true),
        matching: find.byType(InkWell),
      )
      .first,
);

bool _shows(String text) =>
    find.textContaining(text, findRichText: true).evaluate().isNotEmpty;

void main() {
  testWidgets('sections start collapsed and open on tap', (tester) async {
    await tester.pumpWidget(_app(ValueNotifier(_board)));

    expect(_shows('Fleet working (2)'), isTrue);
    expect(_shows('Private channels (1)'), isTrue);
    expect(_shows('Footer'), isTrue);
    expect(_shows('model routing pilot'), isFalse);
    expect(_shows(':::'), isFalse);

    await _tapTitle(tester, 'Fleet working');
    await tester.pump();

    expect(_shows('model routing pilot'), isTrue);
    expect(_shows('personal website'), isFalse);
    expect(
      tester.getSemantics(
        find
            .ancestor(
              of: find.textContaining('Fleet working', findRichText: true),
              matching: find.byType(Semantics),
            )
            .first,
      ),
      matchesSemantics(
        isButton: true,
        hasExpandedState: true,
        isExpanded: true,
        hasTapAction: true,
        isFocusable: true,
        hasFocusAction: true,
        label: 'Fleet working (2)',
        textDirection: TextDirection.ltr,
      ),
    );
  });

  testWidgets('an open section stays open across an in-place edit', (
    tester,
  ) async {
    final content = ValueNotifier(_board);
    await tester.pumpWidget(_app(content));
    await _tapTitle(tester, 'Fleet working');
    await tester.pump();

    content.value = _board.replaceFirst(
      'openclaw upgrade',
      'openclaw upgrade done',
    );
    await tester.pump();

    expect(_shows('openclaw upgrade done'), isTrue);
    expect(_shows('personal website'), isFalse);
  });

  testWidgets('a bold-composed heading section opens on tap', (tester) async {
    await tester.pumpWidget(
      _app(
        ValueNotifier('**:::details ## Heading (1)**\n**- three**\n**:::**'),
      ),
    );
    expect(_shows('three'), isFalse);
    await _tapTitle(tester, 'Heading');
    await tester.pump();
    expect(_shows('three'), isTrue);
  });

  testWidgets('a heading title exposes one labelled, inert-content toggle', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    await tester.pumpWidget(
      _app(
        ValueNotifier(
          ':::details ## [docs](https://example.com) **now**\nbody\n:::',
        ),
      ),
    );
    final toggle = find
        .ancestor(
          of: find.textContaining('now', findRichText: true),
          matching: find.byType(InkWell),
        )
        .first;
    expect(
      tester.getSemantics(toggle),
      matchesSemantics(
        isButton: true,
        isHeader: true,
        hasExpandedState: true,
        isExpanded: false,
        hasTapAction: true,
        isFocusable: true,
        hasFocusAction: true,
        label: 'docs now',
        textDirection: TextDirection.ltr,
      ),
    );
    // The title's link is neither a second semantics node nor tappable.
    expect(find.bySemanticsLabel(RegExp('docs')), findsOneWidget);
    expect(
      find.descendant(of: toggle, matching: find.byType(IgnorePointer)),
      findsWidgets,
    );
    handle.dispose();
  });
}
