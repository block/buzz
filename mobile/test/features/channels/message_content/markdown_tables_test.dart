import 'package:buzz/features/channels/message_content/markdown_tables.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('separates a table from its caption', () {
    expect(
      separateMarkdownTables('Results:\n| a | b |\n|---|:-:|\n| 1 | 2 |'),
      'Results:\n\n| a | b |\n|---|:-:|\n| 1 | 2 |',
    );
  });

  test('a long blank run in a delimiter row takes linear time', () {
    // Overlapping blank runs retried every split: about a second at 20,000.
    final content = 'caption\n| a |\n|---${' ' * 200000}x|';
    final timer = Stopwatch()..start();
    expect(separateMarkdownTables(content), content);
    expect(timer.elapsedMilliseconds, lessThan(500));
  });
}
