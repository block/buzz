import 'package:buzz/shared/reminders/remind_me_later_sheet.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

const _target = ReminderTarget(
  eventId: 'msg-1',
  channelId: 'chan-1',
  preview: 'hello',
  authorPubkey: 'alice',
);

/// Records createReminder calls; throws when [error] is set.
class _RecordingReminderService extends ReminderService {
  final Object? error;
  final List<int> submittedNotBefore = [];

  _RecordingReminderService({this.error})
    : super(
        signedEventRelay: SignedEventRelay(
          session: RelaySessionNotifier(),
          nsec: nostr.Keys.generate().nsec,
        ),
        crypto: _stubCrypto(),
      );

  @override
  Future<void> createReminder({
    required ReminderTarget target,
    required int notBefore,
    String? note,
  }) async {
    final error = this.error;
    if (error != null) throw error;
    submittedNotBefore.add(notBefore);
  }
}

ReminderCrypto _stubCrypto() {
  final keys = nostr.Keys.generate();
  return ReminderCrypto(keys.nsec, keys.public);
}

Future<void> _pumpSheet(
  WidgetTester tester, {
  required ReminderService service,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      overrides: [reminderServiceProvider.overrideWithValue(service)],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Consumer(
            builder: (context, ref, _) => TextButton(
              onPressed: () => showRemindMeLaterSheet(
                context: context,
                ref: ref,
                target: _target,
              ),
              child: const Text('open'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
}

void main() {
  group('showRemindMeLaterSheet', () {
    testWidgets('cancelling the custom date picker keeps the reminder sheet '
        'open', (tester) async {
      final service = _RecordingReminderService();
      await _pumpSheet(tester, service: service);

      await tester.ensureVisible(find.text('Pick a date & time'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Pick a date & time'));
      await tester.pumpAndSettle();

      // Native date picker is up; cancel it.
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();

      // The preset sheet stays open for a retry, and nothing was submitted.
      expect(find.text('Remind me about this message'), findsOneWidget);
      expect(find.text('Pick a date & time'), findsOneWidget);
      expect(service.submittedNotBefore, isEmpty);
    });

    testWidgets('preset submission failure shows stable copy without the '
        'raw error', (tester) async {
      final service = _RecordingReminderService(
        error: Exception('nip44 conversation key mismatch'),
      );
      await _pumpSheet(tester, service: service);

      await tester.tap(find.text('In 1 hour'));
      await tester.pumpAndSettle();

      expect(find.text('Couldn’t set reminder. Try again.'), findsOneWidget);
      expect(
        find.textContaining('nip44 conversation key mismatch'),
        findsNothing,
      );
    });

    testWidgets('preset submission success confirms with a snackbar', (
      tester,
    ) async {
      final service = _RecordingReminderService();
      await _pumpSheet(tester, service: service);

      await tester.tap(find.text('In 1 hour'));
      await tester.pumpAndSettle();

      expect(service.submittedNotBefore, hasLength(1));
      expect(find.text('Reminder set'), findsOneWidget);
    });
  });
}
