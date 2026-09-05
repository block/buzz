import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/message_content.dart';
import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('namesake chips resolve exact tagged keys, not tag order', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final first = 'a' * 64;
    final second = 'b' * 64;
    String? tapped;
    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: MessageContent(
          content: '@Scout @Scout ($second)',
          mentionNames: {second: 'Scout', first: 'Scout'},
          onMentionTap: (key) => tapped = key,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Scout'));
    expect(tapped, first);
    await tester.tap(find.text('Scout (bbbbbbbb…bbbb)'));
    expect(tapped, second);
    expect(find.bySemanticsLabel(RegExp('Scout.*$second')), findsOneWidget);
    expect(tester.takeException(), isNull);
    semantics.dispose();
  });
}
