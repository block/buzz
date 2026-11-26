import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/push/notification_body.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  // The contract every platform runs: docs/adr/0002-notification-message-text.md.
  final fixture =
      jsonDecode(
            File('../test-fixtures/notification-body.json').readAsStringSync(),
          )
          as Map<String, dynamic>;
  final cases = fixture['cases'] as List;

  test('the fixture is the one this formatter was written for', () {
    expect(fixture['schemaVersion'], 1);
    expect(notificationBodyLimit, fixture['maxLength']);
    expect(cases, isNotEmpty);
  });

  for (final testCase in cases) {
    test('notification-body fixture: ${testCase['name']}', () {
      expect(
        notificationBody(testCase['input'] as String),
        testCase['expected'] as String,
      );
    });
  }
}
