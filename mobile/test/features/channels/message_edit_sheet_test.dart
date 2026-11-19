import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

const _channelId = 'channel-1';
final _bob = 'b' * 64;
final _alice = 'a' * 64;
final _quinn = UserProfile(pubkey: 'c' * 64, displayName: 'Quinn');

final _channel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 3,
  isMember: true,
);

final _members = [
  for (final (key, name) in [(_bob, 'Bob'), (_alice, 'Alice')])
    ChannelMember(
      pubkey: key,
      displayName: name,
      role: 'member',
      joinedAt: DateTime(2025),
    ),
];

class _ChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [_channel];
}

class _RelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => RelayConfig(
    baseUrl: 'http://localhost:3000',
    nsec: nostr.Keys.generate().nsec,
  );
}

/// Records the edit the sheet saves; the relay write is the production
/// path's, unchanged by this change.
class _RecordingActions implements ChannelActions {
  final edits =
      <
        ({
          String channelId,
          String eventId,
          String content,
          List<List<String>> tags,
        })
      >[];

  @override
  Future<void> editMessage({
    required String channelId,
    required String eventId,
    required String content,
    List<List<String>> mediaTags = const [],
  }) async {
    edits.add((
      channelId: channelId,
      eventId: eventId,
      content: content,
      tags: mediaTags,
    ));
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

TimelineMessage _message(String content, {List<String> mentions = const []}) =>
    TimelineMessage(
      id: 'msg-1',
      pubkey: 'self',
      createdAt: 1000,
      content: content,
      tags: [
        for (final key in mentions) ['p', key],
      ],
      mentionPubkeys: mentions,
    );

Future<_RecordingActions> _openEditSheet(
  WidgetTester tester, {
  required TimelineMessage message,
  Future<List<UserProfile>> Function(String query)? searchPeople,
}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  final actions = _RecordingActions();
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
        relayClientProvider.overrideWithValue(
          RelayClient(baseUrl: 'http://localhost:3000'),
        ),
        currentPubkeyProvider.overrideWith((ref) => 'self'),
        channelsProvider.overrideWith(_ChannelsNotifier.new),
        channelMembersProvider(
          _channelId,
        ).overrideWith((ref) => Future.value(_members)),
        agentDirectoryProvider.overrideWith((ref) async => const []),
        agentOwnersProvider.overrideWith((ref) async => const {}),
        channelActionsProvider.overrideWithValue(actions),
        if (searchPeople != null)
          mentionUserSearchProvider.overrideWith((ref, query) async {
            await Future<void>.delayed(mentionSearchDebounce);
            final people = await searchPeople(query.trim());
            ref.read(mentionSearchCacheProvider).put(query.trim(), people);
            return people;
          }),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () => showMessageEditSheet(
                context: context,
                message: message,
                channelId: _channelId,
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
  return actions;
}

void main() {
  final field = find.byKey(const ValueKey('message-edit-field'));
  final personChip = find.byKey(const ValueKey('composer-person-mention-chip'));
  final popover = find.byKey(const ValueKey('mention-suggestions-popover'));
  Finder row(String name) =>
      find.descendant(of: popover, matching: find.text(name));
  TextEditingController controllerOf(WidgetTester tester) =>
      tester.widget<TextField>(field).controller!;

  /// Lets a debounced directory search for the current query fire.
  Future<void> pause(WidgetTester tester) async {
    await tester.pump();
    await tester.pump(mentionSearchDebounce * 2);
  }

  testWidgets(
    'editing a message offers the mention chooser and saves prose only',
    (tester) async {
      final original = 'The agenda is ready @Alice';
      final searched = <String>[];
      final actions = await _openEditSheet(
        tester,
        message: _message(original, mentions: [_alice]),
        searchPeople: (query) async {
          searched.add(query);
          return 'quinn'.startsWith(query.toLowerCase()) ? [_quinn] : [];
        },
      );
      final controller = controllerOf(tester);

      // The message's existing mention opens as a chip bound to Alice.
      expect(controller.text, original);
      expect(personChip, findsOneWidget);
      expect(find.text('Edit message'), findsOneWidget);

      // Typing `@bo` opens the composer's chooser; a tap inserts a chip.
      await tester.enterText(field, '$original cc @bo');
      await tester.pumpAndSettle();
      expect(row('Bob'), findsOneWidget);
      await tester.tap(row('Bob'));
      await tester.pumpAndSettle();
      expect(controller.text, '$original cc @Bob ');
      expect(personChip, findsNWidgets(2));
      expect(popover, findsNothing);

      // The chooser follows the field's focus (recommendation 8).
      await tester.enterText(field, '$original cc @Bob and @al');
      await tester.pumpAndSettle();
      expect(row('Alice'), findsOneWidget);
      tester.widget<TextField>(field).focusNode!.unfocus();
      await tester.pumpAndSettle();
      expect(popover, findsNothing);
      expect(controller.text, '$original cc @Bob and @al');
      tester.widget<TextField>(field).focusNode!.requestFocus();
      await tester.pumpAndSettle();
      expect(row('Alice'), findsOneWidget);

      // Closing punctuation selects an exact name as Space does.
      await tester.enterText(field, '$original cc @Bob and @alice');
      await tester.pump();
      await tester.enterText(field, '$original cc @Bob and @alice,');
      await tester.pumpAndSettle();
      expect(controller.text, '$original cc @Bob and @Alice,');
      expect(personChip, findsNWidgets(3));

      // A person the directory finds is picked the same way.
      await tester.enterText(field, '$original cc @Bob @qu');
      await pause(tester);
      expect(searched, contains('qu'));
      expect(row('Quinn'), findsOneWidget);
      expect(row('not in channel'), findsOneWidget);
      await tester.tap(row('Quinn'));
      await tester.pumpAndSettle();
      expect(controller.text, '$original cc @Bob @Quinn ');
      expect(personChip, findsNWidgets(3));

      // Saving writes the text with the picks and adds no recipient: the
      // edit carries the tags this path always wrote, never a `p` tag.
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(actions.edits, hasLength(1));
      final edit = actions.edits.single;
      expect(edit.channelId, _channelId);
      expect(edit.eventId, 'msg-1');
      expect(edit.content, '$original cc @Bob @Quinn');
      expect(edit.tags.where((tag) => tag.first == 'p'), isEmpty);
      expect(find.text('Edit message'), findsNothing);
    },
  );

  testWidgets('an unchanged or emptied edit saves nothing', (tester) async {
    final actions = await _openEditSheet(
      tester,
      message: _message('hello @Bob', mentions: [_bob]),
    );
    expect(personChip, findsOneWidget);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(actions.edits, isEmpty);
    expect(find.text('Edit message'), findsNothing);

    await _openEditSheet(tester, message: _message('hello'));
    await tester.enterText(field, '   ');
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(find.text('Edit message'), findsNothing);
  });

  testWidgets(
    'the sheet fits a small phone with the keyboard and chooser open',
    (tester) async {
      tester.view.physicalSize = const Size(375, 667);
      tester.view.devicePixelRatio = 1;
      tester.view.viewInsets = const FakeViewPadding(bottom: 300);
      addTearDown(tester.view.reset);
      await _openEditSheet(tester, message: _message('one\ntwo\nthree\nfour'));
      final controller = controllerOf(tester);
      await tester.enterText(field, '${controller.text}\nfive @bo');
      await tester.pumpAndSettle();
      expect(row('Bob'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.ensureVisible(find.text('Save'));
      await tester.pumpAndSettle();
      expect(find.text('Save').hitTestable(), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
}
