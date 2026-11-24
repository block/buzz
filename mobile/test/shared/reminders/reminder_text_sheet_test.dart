import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/reminders/reminder_text_sheet.dart';
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

/// Records the writes the sheets ask for; throws when [error] is set.
class _RecordingReminderService extends ReminderService {
  final Object? error;
  final created = <({String text, int notBefore})>[];
  final updated =
      <({String dTag, int notBefore, String? note, ReminderTarget? target})>[];

  _RecordingReminderService({this.error})
    : super(
        signedEventRelay: SignedEventRelay(
          session: RelaySessionNotifier(),
          nsec: nostr.Keys.generate().nsec,
        ),
        crypto: _stubCrypto(),
      );

  @override
  Future<void> createTextReminder({
    required String text,
    required int notBefore,
  }) async {
    final error = this.error;
    if (error != null) throw error;
    created.add((text: text, notBefore: notBefore));
  }

  @override
  Future<void> updateReminderNote({
    required String dTag,
    required int notBefore,
    required String? note,
    ReminderTarget? target,
  }) async {
    final error = this.error;
    if (error != null) throw error;
    updated.add((dTag: dTag, notBefore: notBefore, note: note, target: target));
  }
}

ReminderCrypto _stubCrypto() {
  final keys = nostr.Keys.generate();
  return ReminderCrypto(keys.nsec, keys.public);
}

Future<void> _pump(
  WidgetTester tester, {
  required ReminderService service,
  required void Function(BuildContext, WidgetRef) open,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      overrides: [reminderServiceProvider.overrideWithValue(service)],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Consumer(
            builder: (context, ref, _) => TextButton(
              onPressed: () => open(context, ref),
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

Future<void> _pumpNew(
  WidgetTester tester,
  ReminderService service, {
  VoidCallback? onSaved,
}) => _pump(
  tester,
  service: service,
  open: (context, ref) =>
      showNewReminderSheet(context: context, ref: ref, onSaved: onSaved),
);

Future<void> _pumpEdit(
  WidgetTester tester,
  ReminderService service,
  EditableReminder reminder,
) => _pump(
  tester,
  service: service,
  open: (context, ref) =>
      showEditReminderSheet(context: context, ref: ref, reminder: reminder),
);

final _textField = find.byKey(const ValueKey('reminder-text-field'));

FilledButton _button(WidgetTester tester, String label) =>
    tester.widget<FilledButton>(find.widgetWithText(FilledButton, label));

void main() {
  group('showNewReminderSheet', () {
    testWidgets('asks for required reminder text, then when, and keeps '
        'Set reminder off until the text is not blank', (tester) async {
      await _pumpNew(tester, _RecordingReminderService());

      expect(find.text('New reminder'), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('Reminder text')).dy,
        lessThan(tester.getTopLeft(find.text('When')).dy),
      );
      expect(_button(tester, 'Set reminder').onPressed, isNull);

      await tester.enterText(_textField, '   \n ');
      await tester.pump();
      expect(_button(tester, 'Set reminder').onPressed, isNull);

      await tester.enterText(_textField, 'Call the dentist');
      await tester.pump();
      expect(_button(tester, 'Set reminder').onPressed, isNotNull);
    });

    testWidgets('saves the trimmed text for the chosen time and confirms', (
      tester,
    ) async {
      final service = _RecordingReminderService();
      var saved = 0;
      await _pumpNew(tester, service, onSaved: () => saved++);

      await tester.enterText(_textField, '  Call the dentist\nBook a visit ');
      await tester.tap(find.text('When'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('In 3 hours'));
      await tester.pumpAndSettle();
      expect(find.text('In 3 hours'), findsOneWidget);

      final before = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      await tester.tap(find.widgetWithText(FilledButton, 'Set reminder'));
      await tester.pumpAndSettle();

      final created = service.created.single;
      expect(created.text, 'Call the dentist\nBook a visit');
      expect(created.notBefore, greaterThanOrEqualTo(before + 3 * 60 * 60));
      expect(saved, 1);
      expect(find.text('Reminder set'), findsOneWidget);
      expect(find.text('New reminder'), findsNothing);
    });

    testWidgets('a failed save keeps the sheet and the text, with stable '
        'copy', (tester) async {
      final service = _RecordingReminderService(
        error: Exception('nip44 conversation key mismatch'),
      );
      var saved = 0;
      await _pumpNew(tester, service, onSaved: () => saved++);

      await tester.enterText(_textField, 'Call the dentist');
      await tester.pump();
      await tester.tap(find.widgetWithText(FilledButton, 'Set reminder'));
      await tester.pumpAndSettle();

      expect(
        find.text('The reminder could not be saved. Try again.'),
        findsOneWidget,
      );
      expect(find.textContaining('nip44'), findsNothing);
      expect(find.text('Call the dentist'), findsOneWidget);
      expect(_button(tester, 'Set reminder').onPressed, isNotNull);
      expect(saved, 0);
    });
  });

  group('showEditReminderSheet', () {
    testWidgets('a message reminder offers an optional note without time '
        'choices, and clearing it saves with the target and time kept', (
      tester,
    ) async {
      final service = _RecordingReminderService();
      await _pumpEdit(tester, service, (
        dTag: 'reminder-d1',
        notBefore: 1753000000,
        target: _target,
        note: 'Reply after standup',
      ));

      expect(find.text('Edit reminder'), findsOneWidget);
      expect(find.text('Note (optional)'), findsOneWidget);
      expect(find.text('Reply after standup'), findsOneWidget);
      expect(find.text('When'), findsNothing);
      // Nothing changed yet.
      expect(_button(tester, 'Save').onPressed, isNull);

      await tester.enterText(_textField, '');
      await tester.pump();
      expect(_button(tester, 'Save').onPressed, isNotNull);
      await tester.tap(find.widgetWithText(FilledButton, 'Save'));
      await tester.pumpAndSettle();

      final updated = service.updated.single;
      expect(updated.dTag, 'reminder-d1');
      expect(updated.notBefore, 1753000000);
      expect(updated.target, _target);
      expect(updated.note, isEmpty);
      expect(find.text('Reminder updated'), findsOneWidget);
    });

    testWidgets('a written reminder needs its text and saves it without a '
        'target', (tester) async {
      final service = _RecordingReminderService();
      await _pumpEdit(tester, service, (
        dTag: 'reminder-d2',
        notBefore: 1753000000,
        target: null,
        note: 'Call the dentist',
      ));

      expect(find.text('Reminder text'), findsOneWidget);
      expect(find.text('When'), findsNothing);

      await tester.enterText(_textField, '  ');
      await tester.pump();
      expect(_button(tester, 'Save').onPressed, isNull);

      await tester.enterText(_textField, 'Call the dentist tomorrow');
      await tester.pump();
      await tester.tap(find.widgetWithText(FilledButton, 'Save'));
      await tester.pumpAndSettle();

      final updated = service.updated.single;
      expect(updated.dTag, 'reminder-d2');
      expect(updated.notBefore, 1753000000);
      expect(updated.target, isNull);
      expect(updated.note, 'Call the dentist tomorrow');
    });

    testWidgets('a failed save keeps the edited text', (tester) async {
      final service = _RecordingReminderService(error: Exception('offline'));
      await _pumpEdit(tester, service, (
        dTag: 'reminder-d1',
        notBefore: 1753000000,
        target: _target,
        note: null,
      ));

      await tester.enterText(_textField, 'Reply after standup');
      await tester.pump();
      await tester.tap(find.widgetWithText(FilledButton, 'Save'));
      await tester.pumpAndSettle();

      expect(
        find.text('The reminder could not be saved. Try again.'),
        findsOneWidget,
      );
      expect(find.text('Reply after standup'), findsOneWidget);
    });
  });
}
