import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/app_list.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

const _channelId = 'channel-id';

Channel _channel({String visibility = 'open', String type = 'stream'}) =>
    Channel(
      id: _channelId,
      name: 'general',
      channelType: type,
      visibility: visibility,
      description: '',
      createdBy: 'owner',
      createdAt: DateTime(2025),
      memberCount: 1,
      isMember: true,
    );

/// Relay that serves one channel's kind:39000 and the community roster, and
/// applies accepted kind:9002 admission changes, unless told to accept without
/// applying, lose the acknowledgement, or reject.
class _AdmissionRelay extends RelaySessionNotifier {
  _AdmissionRelay({
    required this.channelType,
    required this.visibility,
    required this.member,
    required this.communityRole,
  });

  final String channelType;
  String visibility;
  final String member;
  final String communityRole;
  String joinRole = 'member';
  bool autoJoin = false;
  bool archived = false;
  bool applies = true;
  bool losesAck = false;
  bool rejects = false;
  bool disconnected = false;
  bool rosterFails = false;
  final changes = <List<List<String>>>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.kind == 9002) {
      // Like the real session, a disconnected one sends nothing.
      if (disconnected) throw RelayNotConnected();
      changes.add(event.tags);
      if (rejects) throw const RelayEventRejected('restricted: not allowed');
      if (applies) {
        for (final tag in event.tags) {
          if (tag[0] == 'join_role') joinRole = tag[1];
          if (tag[0] == 'auto_join') autoJoin = tag[1] == 'true';
        }
      }
      if (losesAck) throw TimeoutException('not acknowledged');
    }
    return event;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (!filters.any((filter) => filter.kinds.contains(13534))) {
      return const [];
    }
    if (rosterFails) throw Exception('roster unavailable');
    return [
      NostrEvent(
        id: 'roster',
        pubkey: 'relay',
        createdAt: 1,
        kind: 13534,
        tags: [
          ['member', member, communityRole],
        ],
        content: '',
        sig: 'sig',
      ),
    ];
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (!filter.kinds.contains(39000)) return const [];
    final eligible = channelType == 'stream' && visibility == 'open';
    return [
      NostrEvent(
        id: 'metadata',
        pubkey: 'relay',
        createdAt: 1,
        kind: 39000,
        tags: [
          ['d', _channelId],
          ['name', 'general'],
          ['t', channelType],
          [visibility == 'open' ? 'public' : 'private'],
          ['join_role', joinRole],
          if (archived) ['archived', 'true'],
          // Only an eligible channel advertises a stored default flag.
          if (autoJoin && eligible && !archived) ['auto_join'],
        ],
        content: '',
        sig: 'sig',
      ),
    ];
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async {
    onStatusChanged(RelaySubscriptionStatus.ready);
    return () {};
  }
}

class _Sections extends ChannelSectionsNotifier {
  @override
  ChannelSectionsState build() => const ChannelSectionsState(
    isReady: true,
    store: ChannelSectionStore(),
    version: 1,
  );
}

final _member = nostr.Keys.generate();

_AdmissionRelay _relay({
  String type = 'stream',
  String visibility = 'open',
  String communityRole = 'member',
}) => _AdmissionRelay(
  channelType: type,
  visibility: visibility,
  member: _member.public,
  communityRole: communityRole,
);

/// The details page, mounted while [open] is true, under one app-wide scope.
Widget _app({
  required _AdmissionRelay relay,
  required Channel channel,
  required String channelRole,
  ValueNotifier<bool>? open,
}) {
  final keys = _member;
  final page = ChannelDetailsPage(
    channel: channel,
    currentPubkey: keys.public,
    onMemberTap: (_, _) {},
  );
  return ProviderScope(
    retry: (_, _) => null,
    overrides: [
      relaySessionProvider.overrideWith(() => relay),
      currentPubkeyProvider.overrideWith((ref) => keys.public),
      myPubkeyProvider.overrideWithValue(keys.public),
      channelMembersProvider(channel.id).overrideWith(
        (ref) async => [
          ChannelMember(
            pubkey: keys.public,
            role: channelRole,
            joinedAt: DateTime(2025),
          ),
        ],
      ),
      agentOwnersProvider.overrideWithValue(
        const AsyncValue.data(<String, String>{}),
      ),
      channelSectionsProvider.overrideWith(_Sections.new),
      channelActionsProvider.overrideWith(
        (ref) => ChannelActions(
          ref: ref,
          session: relay,
          signedEventRelay: SignedEventRelay(session: relay, nsec: keys.nsec),
          currentPubkey: keys.public,
        ),
      ),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: open == null
          ? page
          : ValueListenableBuilder<bool>(
              valueListenable: open,
              builder: (_, isOpen, _) =>
                  isOpen ? page : const Scaffold(body: SizedBox()),
            ),
    ),
  );
}

final _accessCard = find.byKey(const ValueKey('channel-details-access-card'));
final _defaultSetting = find.byKey(const ValueKey('channel-default-setting'));
final _joinRoleRow = find.byKey(const ValueKey('channel-join-role-row'));
final _checkRow = find.byKey(const ValueKey('channel-access-check'));
final _roleRetry = find.byKey(const ValueKey('channel-community-role-retry'));

Switch _defaultSwitch(WidgetTester tester) => tester.widget<Switch>(
  find.descendant(of: _defaultSetting, matching: find.byType(Switch)),
);

Future<void> _scrollTo(WidgetTester tester, Finder finder) async {
  await tester.scrollUntilVisible(
    finder,
    200,
    scrollable: find.byType(Scrollable).first,
  );
  await tester.pumpAndSettle();
}

Future<void> _checkStatus(WidgetTester tester) async {
  await tester.tap(_checkRow);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('admin sets who joins a channel and makes it a default', (
    tester,
  ) async {
    final relay = _relay(communityRole: 'admin');
    final open = ValueNotifier(true);
    addTearDown(open.dispose);
    await tester.pumpWidget(
      _app(relay: relay, channel: _channel(), channelRole: 'admin', open: open),
    );
    await tester.pumpAndSettle();
    await _scrollTo(tester, _defaultSetting);

    expect(find.text('General access'), findsOneWidget);
    expect(
      find.descendant(of: _joinRoleRow, matching: find.text('Member')),
      findsOneWidget,
    );
    expect(_defaultSwitch(tester).value, isFalse);
    expect(
      tester.getSemantics(_defaultSetting),
      isSemantics(hasToggledState: true, isToggled: false),
    );

    // Tapping the row, not only the switch, makes the channel a default.
    await tester.tap(find.text('Default channel'));
    await tester.pumpAndSettle();
    expect(relay.changes.last, [
      ['h', _channelId],
      ['auto_join', 'true'],
    ]);
    expect(_defaultSwitch(tester).value, isTrue);
    expect(
      tester.getSemantics(_defaultSetting),
      isSemantics(hasToggledState: true, isToggled: true),
    );
    expect(_checkRow, findsNothing);

    // A lost acknowledgement may still have saved the change: it is held as
    // uncertain, blocks every further change, and a read confirms it.
    relay.losesAck = true;
    await tester.tap(_joinRoleRow);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('channel-join-role-guest')));
    await tester.pumpAndSettle();
    expect(relay.changes.last, [
      ['h', _channelId],
      ['join_role', 'guest'],
    ]);
    expect(find.textContaining('couldn’t confirm the change'), findsOneWidget);
    expect(_checkRow, findsOneWidget);
    expect(tester.widget<AppListRow>(_joinRoleRow).onTap, isNull);
    await tester.tap(find.text('Default channel'));
    await tester.pumpAndSettle();
    // The actions refuse to publish too, whoever calls them.
    final actions = ProviderScope.containerOf(
      tester.element(_accessCard),
    ).read(channelActionsProvider);
    await expectLater(
      actions.setDefaultChannel(_channelId, enabled: false),
      throwsA(isA<ChannelChangeUnconfirmed>()),
    );
    expect(relay.changes, hasLength(2));
    relay.losesAck = false;
    await _checkStatus(tester);
    expect(_checkRow, findsNothing);
    expect(
      find.descendant(of: _joinRoleRow, matching: find.text('Guest')),
      findsOneWidget,
    );
    expect(relay.changes, hasLength(2));

    // An accepted change the relay's metadata does not show is never
    // reported as saved, and survives closing and reopening the page.
    relay.applies = false;
    await tester.tap(
      find.descendant(of: _defaultSetting, matching: find.byType(Switch)),
    );
    await tester.pumpAndSettle();
    expect(relay.changes.last, [
      ['h', _channelId],
      ['auto_join', 'false'],
    ]);
    expect(_defaultSwitch(tester).value, isTrue);
    open.value = false;
    await tester.pumpAndSettle();
    open.value = true;
    await tester.pumpAndSettle();
    await _scrollTo(tester, _defaultSetting);
    expect(_checkRow, findsOneWidget);
    expect(_defaultSwitch(tester).onChanged, isNull);
    await tester.tap(find.text('Default channel'));
    await tester.pumpAndSettle();
    expect(relay.changes, hasLength(3));

    // Archived and private metadata keep the stored flag without showing
    // it, and open metadata still shows it on: none of them confirm "off".
    relay.archived = true;
    await _checkStatus(tester);
    expect(_checkRow, findsOneWidget);
    relay
      ..archived = false
      ..visibility = 'private';
    await _checkStatus(tester);
    expect(_checkRow, findsOneWidget);
    relay.visibility = 'open';
    await _checkStatus(tester);
    expect(_checkRow, findsOneWidget);

    // Only eligible metadata with the flag cleared resolves it.
    relay.autoJoin = false;
    await _checkStatus(tester);
    expect(_checkRow, findsNothing);
    expect(_defaultSwitch(tester).value, isFalse);
    expect(relay.changes, hasLength(3));

    // A rejection is definite: nothing is held, and the user can try again.
    relay.rejects = true;
    await tester.tap(find.text('Default channel'));
    await tester.pumpAndSettle();
    expect(relay.changes, hasLength(4));
    expect(
      find.text('Couldn’t change channel access. Try again.'),
      findsOneWidget,
    );
    expect(_checkRow, findsNothing);
    expect(_defaultSwitch(tester).onChanged, isNotNull);

    // So is a change made while disconnected, and it works after reconnect.
    relay
      ..rejects = false
      ..disconnected = true;
    await tester.tap(find.text('Default channel'));
    await tester.pumpAndSettle();
    expect(relay.changes, hasLength(4));
    expect(
      find.text('Couldn’t change channel access. Try again.'),
      findsOneWidget,
    );
    expect(_checkRow, findsNothing);
    relay.disconnected = false;
    await tester.tap(find.text('Default channel'));
    await tester.pumpAndSettle();
    expect(relay.changes, hasLength(5));
    expect(_checkRow, findsNothing);
    expect(_defaultSwitch(tester).value, isTrue);
  });

  testWidgets('a failed community role read explains and retries in place', (
    tester,
  ) async {
    final relay = _relay(communityRole: 'admin')..rosterFails = true;
    await tester.pumpWidget(
      _app(relay: relay, channel: _channel(), channelRole: 'member'),
    );
    await tester.pumpAndSettle();
    await _scrollTo(tester, _roleRetry);
    expect(_defaultSetting, findsNothing);

    relay.rosterFails = false;
    await tester.tap(_roleRetry);
    await tester.pumpAndSettle();
    expect(_roleRetry, findsNothing);
    expect(_defaultSetting, findsOneWidget);
    expect(relay.changes, isEmpty);
  });

  testWidgets('shows only the controls each role may use', (tester) async {
    for (final (visibility, type, channelRole, communityRole, joinAs, default_)
        in const [
          // Channel admin, community member: join role only.
          ('open', 'stream', 'admin', 'member', true, false),
          // Community admin, channel member: default channel only.
          ('open', 'stream', 'member', 'owner', false, true),
          // A forum cannot be a default channel.
          ('open', 'forum', 'member', 'admin', false, false),
          // A private channel has no general access.
          ('private', 'stream', 'admin', 'owner', false, false),
          // Neither role manages access.
          ('open', 'stream', 'member', 'member', false, false),
        ]) {
      final relay = _relay(
        type: type,
        visibility: visibility,
        communityRole: communityRole,
      );
      await tester.pumpWidget(
        _app(
          relay: relay,
          channel: _channel(visibility: visibility, type: type),
          channelRole: channelRole,
        ),
      );
      await tester.pumpAndSettle();
      final reason = '$visibility $type $channelRole $communityRole';
      expect(
        _accessCard,
        joinAs || default_ ? findsOneWidget : findsNothing,
        reason: reason,
      );
      expect(
        _joinRoleRow,
        joinAs ? findsOneWidget : findsNothing,
        reason: reason,
      );
      expect(
        _defaultSetting,
        default_ ? findsOneWidget : findsNothing,
        reason: reason,
      );
      expect(relay.changes, isEmpty, reason: reason);
      await tester.pumpWidget(const SizedBox());
    }
  });
}
