import 'package:buzz/shared/messages/message_action.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('recognizes only an authored leading /me command with an action', () {
    final cases = <String, String?>{
      '/me passes out': 'passes out',
      '/me  eats a drumstick  ': 'eats a drumstick',
      '/me waves\nand sits down': 'waves\nand sits down',
      '/me': null,
      '/me   ': null,
      '/meeting': null,
      '/ME waves': null,
      ' /me waves': null,
      '/me\twaves': null,
      r'\/me waves': null,
      '`/me waves`': null,
      '> /me waves': null,
      '```\n/me waves\n```': null,
      'Try /me waves': null,
    };
    for (final entry in cases.entries) {
      expect(messageAction(entry.key), entry.value, reason: entry.key);
    }
  });
}
