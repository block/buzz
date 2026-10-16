import 'package:buzz/features/channels/message_content/link_reference_definitions.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('drops an unused definition that opens the message', () {
    expect(
      stripUnusedLinkReferenceDefinitions(
        '[AGENT-RESULT:claude.alice:update-1]: #\n\nReview-ready in MR !12.',
      ),
      'Review-ready in MR !12.',
    );
  });

  test('drops an unused definition in the middle of the message', () {
    expect(
      stripUnusedLinkReferenceDefinitions(
        'First paragraph.\n\n[notes]: https://example.com\n\nSecond paragraph.',
      ),
      'First paragraph.\n\n\nSecond paragraph.',
    );
  });

  test('keeps a definition that a reference uses', () {
    const content =
        'See [the docs][guide] and [Guide].\n\n'
        '[guide]: https://example.com/guide "Guide"';
    expect(stripUnusedLinkReferenceDefinitions(content), content);
  });

  test('keeps a definition referenced through a collapsed reference', () {
    const content = 'Read [Guide][] first.\n\n[guide]: https://example.com';
    expect(stripUnusedLinkReferenceDefinitions(content), content);
  });

  test('keeps definition-like lines inside fenced code', () {
    const content = '```markdown\n[label]: https://example.com\n```';
    expect(stripUnusedLinkReferenceDefinitions(content), content);
  });

  test('keeps a line that continues a paragraph', () {
    const content = 'Paragraph text\n[label]: https://example.com';
    expect(stripUnusedLinkReferenceDefinitions(content), content);
  });

  test('keeps lines that only look like definitions', () {
    const content = '[label]:\n\n[ ]: https://example.com\n\n[a]: b c';
    expect(stripUnusedLinkReferenceDefinitions(content), content);
  });

  test('handles CRLF line endings', () {
    expect(
      stripUnusedLinkReferenceDefinitions(
        '[AGENT-ACTIVITY:started:claude.alice:u-2]: #\r\n\r\n**Title**\r\nText',
      ),
      '**Title**\r\nText',
    );
  });

  test('returns content without definitions unchanged', () {
    const content = 'A [link](https://example.com) and `[x]: y` in code.';
    expect(stripUnusedLinkReferenceDefinitions(content), content);
  });
}
