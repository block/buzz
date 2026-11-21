import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/compose_bar.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/message_mention_pill.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/utils/stylus_handwriting.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/widget_helpers.dart';

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

/// Sheet builds read the cached roster once each, so its reads count them.
var _rosterReads = 0;

class _ChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [_channel];

  @override
  List<ChannelMember> cachedMembersForChannel(String channelId) {
    _rosterReads++;
    return super.cachedMembersForChannel(channelId);
  }
}

/// The identity link an edit writes for [label] naming [key].
String _link(String label, String key) {
  final npub = nostr.Nip19.encode(prefix: nostr.Nip19Prefix.npub, data: key);
  return '[$label](nostr:$npub)';
}

/// Draws [edit] as the timeline does: an edit's own tags replace the
/// original's, so nothing but the body can name a person.
Future<List<String>> _pumpEditedBubble(
  WidgetTester tester,
  ({String channelId, String eventId, String content, List<List<String>> tags})
  edit,
) async {
  final taps = <String>[];
  await tester.pumpWidget(const SizedBox.shrink());
  await tester.pumpWidget(
    WidgetHelpers.testable(
      child: MessageContent(
        content: edit.content,
        tags: [
          ['h', edit.channelId],
          ['e', edit.eventId],
          ...edit.tags,
        ],
        edited: true,
        onMentionTap: taps.add,
      ),
    ),
  );
  await tester.pumpAndSettle();
  return taps;
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
  Future<List<ChannelMember>>? members,
}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  final actions = _RecordingActions();
  await tester.pumpWidget(const SizedBox.shrink());
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
        ).overrideWith((ref) => members ?? Future.value(_members)),
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
    'editing a message offers the mention chooser and saves picks as links',
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
      // The pick is a stylus press: the chooser is inside the field's tap
      // region, so the press keeps focus and the row is there at release.
      final focusNode = tester.widget<TextField>(field).focusNode!;
      for (final kind in [PointerDeviceKind.stylus, PointerDeviceKind.mouse]) {
        await tester.enterText(field, '$original cc @bo');
        await tester.pumpAndSettle();
        final press = await tester.startGesture(
          tester.getCenter(row('Bob')),
          kind: kind,
        );
        await tester.pumpAndSettle();
        expect(focusNode.hasFocus, isTrue, reason: '$kind');
        expect(row('Bob'), findsOneWidget, reason: '$kind');
        await press.up();
        await press.removePointer();
        await tester.pumpAndSettle();
        expect(controller.text, '$original cc @Bob ', reason: '$kind');
      }
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

      // Saving writes each chip as an identity link to its exact key and
      // adds no recipient: the edit carries the tags this path always wrote,
      // never a `p` tag.
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(actions.edits, hasLength(1));
      final edit = actions.edits.single;
      expect(edit.channelId, _channelId);
      expect(edit.eventId, 'msg-1');
      expect(
        edit.content,
        'The agenda is ready ${_link('@Alice', _alice)} cc '
        '${_link('@Bob', _bob)} ${_link('@Quinn', _quinn.pubkey)}',
      );
      expect(edit.tags.where((tag) => tag.first == 'p'), isEmpty);
      expect(find.text('Edit message'), findsNothing);

      // The bubble draws each link as that person's pill, with no tag.
      final taps = await _pumpEditedBubble(tester, edit);
      for (final name in ['Alice', 'Bob', 'Quinn']) {
        await tester.tap(find.text(name));
      }
      expect(taps, [_alice, _bob, _quinn.pubkey]);
      expect(find.textContaining('npub', findRichText: true), findsNothing);

      // Reopening the saved edit shows the same three chips and plain names.
      final again = await _openEditSheet(
        tester,
        message: _message(edit.content),
      );
      expect(controllerOf(tester).text, '$original cc @Bob @Quinn');
      expect(personChip, findsNWidgets(3));
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(again.edits, isEmpty);
    },
  );

  testWidgets('a mention inside a web link stays that link\'s text', (
    tester,
  ) async {
    // Review of #9699 (editing a message with a mention inside a web link):
    // the `@Bob` of a link's label is the link's text. It opens as text,
    // and saving leaves the link exactly as it was written.
    const webLink = '[@Bob](https://example.org/bob)';
    var actions = await _openEditSheet(
      tester,
      message: _message('$webLink has notes', mentions: [_bob]),
    );
    expect(controllerOf(tester).text, '$webLink has notes');
    expect(personChip, findsNothing);
    // Unchanged, it saves nothing.
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(actions.edits, isEmpty);
    actions = await _openEditSheet(
      tester,
      message: _message('$webLink has notes', mentions: [_bob]),
    );
    await tester.enterText(field, '$webLink has the notes');
    await tester.pumpAndSettle();
    expect(personChip, findsNothing);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(actions.edits.single.content, '$webLink has the notes');

    // A plain `@Bob` beside the link is still his chip, saved as an
    // identity link; the web link is untouched. Titles, images and
    // destinations are link text as well.
    const linked =
        '$webLink ![@Bob](https://example.org/bob.png) '
        '[site](https://example.org "by @Bob")';
    actions = await _openEditSheet(
      tester,
      message: _message('$linked and @Bob', mentions: [_bob]),
    );
    expect(controllerOf(tester).text, '$linked and @Bob');
    expect(personChip, findsOneWidget);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(actions.edits, isEmpty);
    actions = await _openEditSheet(
      tester,
      message: _message('$linked and @Bob', mentions: [_bob]),
    );
    await tester.enterText(field, '$linked and @Bob too');
    await tester.pumpAndSettle();
    expect(personChip, findsOneWidget);

    // A pick made inside a link's label stays text, so no link is written
    // inside a link.
    await tester.enterText(field, '$linked and @Bob too [ask @al');
    await tester.pumpAndSettle();
    await tester.tap(row('Alice'));
    await tester.pumpAndSettle();
    expect(controllerOf(tester).text, '$linked and @Bob too [ask @Alice ');
    expect(personChip, findsNWidgets(2));
    await tester.enterText(
      field,
      '$linked and @Bob too [ask @Alice ](https://example.org/ask)',
    );
    await tester.pumpAndSettle();
    expect(personChip, findsOneWidget);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    final edit = actions.edits.single;
    final saved =
        '$linked and ${_link('@Bob', _bob)} too '
        '[ask @Alice ](https://example.org/ask)';
    expect(edit.content, saved);
    expect(edit.tags.where((tag) => tag.first == 'p'), isEmpty);

    // Reopened, the identity link is Bob's chip again and the web links are
    // text; unchanged, it saves nothing.
    final again = await _openEditSheet(tester, message: _message(saved));
    expect(
      controllerOf(tester).text,
      '$linked and @Bob too [ask @Alice ](https://example.org/ask)',
    );
    expect(personChip, findsOneWidget);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(again.edits, isEmpty);
  });

  testWidgets('an edit seeds a chip only where the bubble draws a pill', (
    tester,
  ) async {
    // Review of #9699 (editing rewrites text the bubble does not draw as a
    // mention): a tagged name inside code of any form, or in a reference
    // link or its definition, is literal text. Each row is a message with
    // Bob tagged; `{bob}` marks the `@Bob` the bubble draws as his pill.
    // `{pill}` marks one the bubble also draws as his pill but other
    // Markdown readers read as a link or code (review 5986525865): it opens
    // as text and an edit leaves it as written.
    final rows = [
      'x ``@Bob`` and ``a ` @Bob`` then {bob}',
      '{bob} ran\n~~~\n@Bob\n~~~\nok',
      '{bob} ran\n````\n@Bob\n```\n@Bob\n````\nok',
      '{bob} ran\n```\n@Bob unclosed',
      '{bob} ran\n~~~\n@Bob unclosed',
      'see [@Bob]\n\n[@Bob]: https://example.org/bob',
      '{bob} see [@Bob] and [notes][@Bob]\n\n[@Bob]: https://example.org/bob',
      // The bubble does not pair a reference with a definition in another
      // case and draws his pill in brackets; standard readers pair them.
      '{bob} see [{pill}]\n\n[@bob]: https://example.org/bob',
      // An indented code block for standard readers, by spaces or a tab.
      '{bob} ran\n\n    {pill} indented\n\nok {bob}',
      '{bob} ran\n\n\t{pill} tabbed',
      // An indented line that continues a paragraph or sits in a list item
      // is prose for every reader.
      '{bob} ran\n    {bob} continues',
      '- item\n\n    {bob} in the item',
      // An identity link in code is raw text there, not a chip.
      '``${_link('@Bob', _bob)}`` and {bob}',
      '~~~\n${_link('@Bob', _bob)}\n~~~\nok',
    ];
    final pill = find.byType(MessageMentionPill);
    for (final row in rows) {
      final original = row
          .replaceAll('{bob}', '@Bob')
          .replaceAll('{pill}', '@Bob');
      final chips = '{bob}'.allMatches(row).length;
      final pills = chips + '{pill}'.allMatches(row).length;

      // The bubble of the message as sent.
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpWidget(
        WidgetHelpers.testable(
          child: MessageContent(
            content: original,
            tags: [
              ['h', _channelId],
              ['p', _bob],
            ],
            mentionNames: {_bob: 'Bob'},
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(pill, findsNWidgets(pills), reason: row);

      // The sheet shows the message as written, with a chip per pill every
      // reader agrees on, and saves nothing unchanged.
      var actions = await _openEditSheet(
        tester,
        message: _message(original, mentions: [_bob]),
      );
      expect(controllerOf(tester).text, original, reason: row);
      expect(personChip, findsNWidgets(chips), reason: row);
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(actions.edits, isEmpty, reason: row);

      // An unrelated edit rewrites only the pills, as identity links.
      actions = await _openEditSheet(
        tester,
        message: _message(original, mentions: [_bob]),
      );
      await tester.enterText(field, 'Edit.\n\n$original');
      await tester.pumpAndSettle();
      expect(personChip, findsNWidgets(chips), reason: row);
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      final edit = actions.edits.single;
      expect(
        edit.content,
        'Edit.\n\n'
        '${row.replaceAll('{bob}', _link('@Bob', _bob)).replaceAll('{pill}', '@Bob')}',
        reason: row,
      );

      // The edited bubble draws the same pills and nothing new: a name
      // left as text is still drawn beside a link that names its person.
      await _pumpEditedBubble(tester, edit);
      expect(pill, findsNWidgets(pills), reason: row);

      // Reopened, it is the same draft and saves nothing unchanged.
      final again = await _openEditSheet(
        tester,
        message: _message(edit.content),
      );
      expect(controllerOf(tester).text, 'Edit.\n\n$original', reason: row);
      expect(personChip, findsNWidgets(chips), reason: row);
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(again.edits, isEmpty, reason: row);
    }
  });

  testWidgets('a name two tagged people share after folding opens as text', (
    tester,
  ) async {
    // Review of #9841: `Sam` and `ſam` fold to one name, as the text scan
    // compares them, so neither tag order may seed a chip for either key.
    final sam = '1' * 64, longS = '2' * 64;
    for (final tagged in [
      [sam, longS],
      [longS, sam],
    ]) {
      await _openEditSheet(
        tester,
        message: _message('ask @Sam', mentions: tagged),
        members: Future.value([
          for (final (key, name) in [(sam, 'Sam'), (longS, '\u017Fam')])
            ChannelMember(
              pubkey: key,
              displayName: name,
              role: 'member',
              joinedAt: DateTime(2025),
            ),
        ]),
      );
      expect(controllerOf(tester).text, 'ask @Sam', reason: '$tagged');
      expect(personChip, findsNothing, reason: '$tagged');
    }
  });

  testWidgets('two picked namesakes save as links and never as raw keys', (
    tester,
  ) async {
    final samA = '1' * 64, samB = '2' * 64;
    final actions = await _openEditSheet(
      tester,
      message: _message('ask'),
      members: Future.value([
        for (final key in [samA, samB])
          ChannelMember(
            pubkey: key,
            displayName: 'Sam',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
      ]),
    );
    final controller = controllerOf(tester);
    // Namesake rows carry a short key; the draft and the chips never do.
    final sams = find.descendant(
      of: popover,
      matching: find.textContaining('Sam · '),
    );
    for (final pick in [0, 1]) {
      await tester.enterText(
        field,
        '${controller.text}${pick == 0 ? ' ' : ''}@sam',
      );
      await tester.pumpAndSettle();
      expect(sams, findsNWidgets(2));
      await tester.tap(sams.at(pick));
      await tester.pumpAndSettle();
    }
    expect(controller.text, 'ask @Sam @Sam ');
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    final edit = actions.edits.single;
    expect(edit.tags.where((tag) => tag.first == 'p'), isEmpty);
    expect(edit.content, isNot(contains(samA)));
    expect(edit.content, isNot(contains(samB)));
    final keys = {
      for (final key in [samA, samB])
        if (edit.content.contains(_link('@Sam', key))) key,
    };
    expect(keys, {samA, samB});

    // Each pill opens its own Sam, and no key is shown.
    final taps = await _pumpEditedBubble(tester, edit);
    expect(find.text('Sam'), findsNWidgets(2));
    await tester.tap(find.text('Sam').first);
    await tester.tap(find.text('Sam').last);
    expect(taps.toSet(), {samA, samB});
    expect(taps.first, isNot(taps.last));
    for (final hidden in ['npub', '1111', '2222']) {
      expect(find.textContaining(hidden, findRichText: true), findsNothing);
    }
  });

  testWidgets('a cleared edit stays empty when a mentioned profile arrives', (
    tester,
  ) async {
    // Untouched, the field gains Bob's chip once his name is known.
    var members = Completer<List<ChannelMember>>();
    await _openEditSheet(
      tester,
      message: _message('hello @Bob', mentions: [_bob]),
      members: members.future,
    );
    expect(controllerOf(tester).text, 'hello @Bob');
    expect(personChip, findsNothing);
    members.complete(_members);
    await tester.pumpAndSettle();
    expect(controllerOf(tester).text, 'hello @Bob');
    expect(personChip, findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();

    // Cleared before the name arrives, it stays as the user left it.
    members = Completer<List<ChannelMember>>();
    final actions = await _openEditSheet(
      tester,
      message: _message('hello @Bob', mentions: [_bob]),
      members: members.future,
    );
    await tester.enterText(field, '');
    await tester.pumpAndSettle();
    members.complete(_members);
    await tester.pumpAndSettle();
    expect(controllerOf(tester).text, isEmpty);
    expect(personChip, findsNothing);
    await tester.tap(find.text('Save'));
    await tester.pumpAndSettle();
    expect(actions.edits, isEmpty);
  });

  testWidgets('the keyboard moving does not rebuild the sheet', (tester) async {
    addTearDown(tester.view.reset);
    await _openEditSheet(tester, message: _message('hello'));
    final before = _rosterReads;
    final top = tester.getTopLeft(field).dy;
    for (final inset in [80.0, 160.0, 240.0, 320.0]) {
      tester.view.viewInsets = FakeViewPadding(
        bottom: inset * tester.view.devicePixelRatio,
      );
      await tester.pump();
    }
    // The field moved up with the keyboard; the sheet built nothing again.
    expect(tester.getTopLeft(field).dy, lessThan(top));
    expect(_rosterReads, before);
  });

  testWidgets('iOS turns stylus handwriting off only while the edit is long', (
    tester,
  ) async {
    bool handwriting() =>
        tester.widget<TextField>(field).stylusHandwritingEnabled;
    await _openEditSheet(
      tester,
      message: _message('a' * (stylusHandwritingMaxLength + 1)),
    );
    expect(handwriting(), isFalse);
    // Opening never measured the message, not even on the first frame.
    expect(
      tester.testTextInput.log.map((call) => call.method),
      isNot(contains('TextInput.setSelectionRects')),
    );
    await tester.enterText(field, 'short');
    await tester.pump();
    expect(handwriting(), isTrue);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('iOS closes a Scribble writing gap left after losing focus', (
    tester,
  ) async {
    Future<void> sendScribble(String method, [List<Object> size = const []]) =>
        tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          SystemChannels.textInput.name,
          SystemChannels.textInput.codec.encodeMethodCall(
            MethodCall('TextInputClient.$method', <Object>[-1, ...size]),
          ),
          (_) {},
        );
    bool hasWritingGap() {
      var found = false;
      tester
          .state<EditableTextState>(find.byType(EditableText))
          .renderEditable
          .text
          ?.visitChildren((span) {
            found = found || span is WidgetSpan;
            return !found;
          });
      return found;
    }

    await _openEditSheet(tester, message: _message('abc'));
    await sendScribble('insertTextPlaceholder', [300, 0]);
    await tester.pump();
    expect(hasWritingGap(), isTrue);
    // When iOS closes the input connection itself, Flutter keeps the gap
    // after the field loses focus, and ignores closing it once handwriting is
    // off. So the edit crossing the cutoff without focus must close it.
    await sendScribble('onConnectionClosed');
    await tester.pump();
    expect(tester.widget<TextField>(field).focusNode!.hasPrimaryFocus, isFalse);
    expect(hasWritingGap(), isTrue);
    controllerOf(tester).text = 'a' * (stylusHandwritingMaxLength + 1);
    await tester.pump();
    expect(hasWritingGap(), isFalse);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

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
