import 'dart:convert';

import 'package:buzz/features/channels/agent_activity/shared_activity_models.dart';
import 'package:buzz/features/channels/agent_activity/shared_activity_summary.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../../helpers/widget_helpers.dart';

void main() {
  testWidgets('renders only neutral safe activity labels', (tester) async {
    final activities = parseSharedActivityFrame(
      jsonEncode({
        'version': 1,
        'activities': [
          _activity(),
          _activity(
            id: 1,
            activityClass: 'tool',
            status: 'completed',
            toolKind: 'search',
            durationMs: 325,
          ),
          _activity(
            id: 2,
            activityClass: 'tool',
            status: 'running',
            toolKind: 'think',
          ),
          _activity(
            id: 3,
            activityClass: 'tool',
            status: 'failed',
            toolKind: 'execute',
            durationMs: 1200,
          ),
          _activity(
            id: 4,
            activityClass: 'turn',
            status: 'cancelled',
            durationMs: 50,
          ),
          _activity(
            id: 5,
            activityClass: 'usage',
            status: 'completed',
            usage: {
              'inputTokens': 100,
              'outputTokens': 25,
              'totalTokens': 125,
              'cacheReadTokens': 20,
              'cacheWriteTokens': 4,
            },
          ),
        ],
      }),
    );

    await tester.pumpWidget(
      WidgetHelpers.testable(
        child: SizedBox(
          height: 600,
          child: SharedActivitySummary(activities: activities),
        ),
      ),
    );

    expect(find.text('Working'), findsWidgets);
    expect(find.text('Search'), findsOneWidget);
    expect(find.text('Execute'), findsOneWidget);
    expect(find.text('Completed'), findsOneWidget);
    expect(find.text('Failed'), findsOneWidget);
    expect(find.text('Cancelled'), findsOneWidget);
    expect(find.text('Duration: 325 ms'), findsOneWidget);
    expect(find.text('Duration: 1.2 s'), findsOneWidget);
    expect(find.text('Per-turn token usage'), findsNWidgets(2));
    expect(find.text('Input: 100'), findsOneWidget);
    expect(find.text('Output: 25'), findsOneWidget);
    expect(find.text('Total: 125'), findsOneWidget);
    expect(find.text('Cache read: 20'), findsOneWidget);
    expect(find.text('Cache write: 4'), findsOneWidget);

    final renderedText = tester
        .widgetList<Text>(find.byType(Text))
        .map((widget) => widget.data ?? '')
        .join(' ')
        .toLowerCase();
    for (final forbidden in [
      'think',
      'thinking',
      'reasoning',
      'chain-of-thought',
      'prompt',
      'arguments',
      'result',
      'error details',
    ]) {
      expect(renderedText, isNot(contains(forbidden)), reason: forbidden);
    }
  });
}

Map<String, Object?> _activity({
  int id = 0,
  String activityClass = 'turn',
  String status = 'started',
  String? toolKind,
  int? durationMs,
  Map<String, Object?>? usage,
}) => {
  'activityId': '00000000-0000-4000-8000-${id.toString().padLeft(12, '0')}',
  'occurredAt': '2026-08-12T12:00:${id.toString().padLeft(2, '0')}Z',
  'activityClass': activityClass,
  'status': status,
  'toolKind': ?toolKind,
  'durationMs': ?durationMs,
  'usage': ?usage,
};
