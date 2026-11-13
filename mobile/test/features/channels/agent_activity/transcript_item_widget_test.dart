import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';
import 'package:buzz/features/channels/agent_activity/transcript_item_widget.dart';
import 'package:buzz/shared/theme/theme.dart';

/// Whether any paragraph on screen draws a link. gpt_markdown resolves link
/// taps by text range, so a link shows up in the span tree only as its style.
bool _rendersLink(WidgetTester tester) {
  // gpt_markdown draws paragraphs with a RichText subclass.
  final paragraphs = find.byWidgetPredicate((widget) => widget is RichText);
  var found = false;
  for (final paragraph in tester.widgetList<RichText>(paragraphs)) {
    paragraph.text.visitChildren((span) {
      found =
          span.style?.decoration?.contains(TextDecoration.underline) ?? false;
      return !found;
    });
    if (found) break;
  }
  return found;
}

Widget _testable(Widget child) {
  return MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(body: SingleChildScrollView(child: child)),
  );
}

void main() {
  // Neither transcript surface passes a link handler, so an autolinked bare
  // URL would draw as a link and then do nothing when tapped. They stay text.
  testWidgets('leaves a bare URL in a message as plain text', (tester) async {
    await tester.pumpWidget(
      _testable(
        TranscriptItemWidget(
          item: MessageItem(
            id: 'm1',
            role: 'assistant',
            title: 'Assistant',
            text: 'Report is at https://example.com/report today.',
            timestamp: '2026-08-29T09:00:00Z',
          ),
        ),
      ),
    );

    expect(
      find.textContaining('https://example.com/report', findRichText: true),
      findsOneWidget,
    );
    expect(_rendersLink(tester), isFalse);
  });

  testWidgets('leaves a bare URL in a thought as plain text', (tester) async {
    await tester.pumpWidget(
      _testable(
        TranscriptItemWidget(
          item: ThoughtItem(
            id: 't1',
            title: 'Thinking',
            text: 'Checking https://example.com/report first.',
            timestamp: '2026-08-29T09:00:00Z',
          ),
        ),
      ),
    );

    expect(
      find.textContaining('https://example.com/report', findRichText: true),
      findsOneWidget,
    );
    expect(_rendersLink(tester), isFalse);
  });

  testWidgets('reads escapes as literal characters', (tester) async {
    await tester.pumpWidget(
      _testable(
        TranscriptItemWidget(
          item: MessageItem(
            id: 'm2',
            role: 'assistant',
            title: 'Assistant',
            text: r'Group as \(a + b\), then ping \@alice.',
            timestamp: '2026-08-29T09:00:00Z',
          ),
        ),
      ),
    );

    expect(
      find.textContaining(
        'Group as (a + b), then ping @alice.',
        findRichText: true,
      ),
      findsOneWidget,
    );
  });
}
