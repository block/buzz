import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:intl/intl.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/fake_relay.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

void main() {
  testWidgets('/remind sets a standalone reminder, or rejects the draft', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final relay = FakeRelay();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ...relay.overrides,
          savedPrefsProvider.overrideWithValue(prefs),
          appLifecycleProvider.overrideWith(_Resumed.new),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: ChannelDetailPage(
            channel: Channel(
              id: _channelId,
              name: 'general',
              channelType: 'stream',
              visibility: 'open',
              description: 'General discussion',
              createdBy: 'abc123',
              createdAt: DateTime(2025),
              memberCount: 5,
              isMember: true,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    String draft() =>
        tester.widget<TextField>(find.byType(TextField)).controller!.text;
    Iterable<NostrEvent> reminders() =>
        relay.published.where((e) => e.kind == EventKind.eventReminder);
    Future<void> submit(String text) async {
      if (find.byType(TextField).evaluate().isEmpty) {
        await tester.tap(find.text('Message #general'));
        await tester.pumpAndSettle();
      }
      await tester.enterText(find.byType(TextField), text);
      await tester.pump();
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
    }

    Future<void> dismissSnackBar() async {
      await tester.pump(const Duration(seconds: 6));
      await tester.pumpAndSettle();
    }

    final earliest = DateTime.now().millisecondsSinceEpoch ~/ 1000 + 20 * 60;
    await submit('/remind me to stretch in 20 minutes');
    final latest = DateTime.now().millisecondsSinceEpoch ~/ 1000 + 20 * 60;

    final event = reminders().single;
    final notBefore = int.parse(event.getTagValue('not_before')!);
    expect(notBefore, inInclusiveRange(earliest, latest));
    final crypto = ReminderCrypto(relay.keys.nsec, relay.pubkey);
    expect(
      crypto.decrypt(event.content),
      '{"note":"stretch","status":"pending"}',
    );
    expect(
      jsonDecode(crypto.decrypt(event.content)),
      isNot(contains('target')),
    );
    final due = DateTime.fromMillisecondsSinceEpoch(notBefore * 1000);
    expect(
      find.text(
        'Reminder set for ${DateFormat("EEE, MMM d 'at' h:mm a").format(due)}',
      ),
      findsOneWidget,
    );
    expect(draft(), isEmpty);
    expect(
      relay.published.where((e) => e.kind == EventKind.streamMessage),
      isEmpty,
      reason: 'a command is never posted as a message',
    );
    await dismissSnackBar();

    await submit('/remind me in 2 hours');
    expect(reminders(), hasLength(1));
    expect(draft(), '/remind me in 2 hours');
    expect(find.textContaining('Say what to be reminded about.'), findsOne);
    expect(find.textContaining('/remind me <what> <when>'), findsOne);
    await dismissSnackBar();

    await submit('/remind @Bob to stretch in 1 hour');
    expect(reminders(), hasLength(1));
    expect(draft(), '/remind @Bob to stretch in 1 hour');
    expect(
      find.textContaining('Reminders are personal. Use /remind me …'),
      findsOneWidget,
    );
  });
}
