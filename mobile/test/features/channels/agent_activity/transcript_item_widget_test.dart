import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gpt_markdown/gpt_markdown.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';
import 'package:buzz/features/channels/agent_activity/transcript_item_widget.dart';
import 'package:buzz/shared/theme/theme.dart';

Widget _testable(Widget child) {
  return MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(body: SingleChildScrollView(child: child)),
  );
}

/// Whether any paragraph in the tree draws a link.
bool _rendersLink(WidgetTester tester) {
  // Not `visitChildren`: it only visits spans that carry their own text, and
  // a link wrapping its parsed label carries none.
  bool walk(InlineSpan span) =>
      span is LinkTextSpan ||
      (span is TextSpan && (span.children?.any(walk) ?? false));

  return tester
      .widgetList<RichText>(
        find.byWidgetPredicate((widget) => widget is RichText),
      )
      .any((rich) => walk(rich.text));
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

    expect(_rendersLink(tester), isFalse);
  });
}
