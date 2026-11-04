import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
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

/// Relay that serves one channel's kind:39000 and applies accepted kind:9002
/// admission changes to it, unless told to accept without applying.
class _AdmissionRelay extends RelaySessionNotifier {
  _AdmissionRelay({required this.channelType, required this.visibility});

  final String channelType;
  final String visibility;
  String joinRole = 'member';
  bool autoJoin = false;
  bool applies = true;
  final changes = <List<List<String>>>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.kind == 9002) {
      changes.add(event.tags);
      if (applies) {
        for (final tag in event.tags) {
          if (tag[0] == 'join_role') joinRole = tag[1];
          if (tag[0] == 'auto_join') autoJoin = tag[1] == 'true';
        }
      }
    }
    return event;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (!filter.kinds.contains(39000)) return const [];
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
          if (autoJoin) ['auto_join'],
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

Widget _app({
  required _AdmissionRelay relay,
  required Channel channel,
  required String channelRole,
  required CommunityMemberRole communityRole,
}) {
  final keys = nostr.Keys.generate();
  return ProviderScope(
    retry: (_, _) => null,
    overrides: [
      relaySessionProvider.overrideWith(() => relay),
      currentPubkeyProvider.overrideWith((ref) => keys.public),
      myPubkeyProvider.overrideWithValue(keys.public),
      currentCommunityRoleProvider.overrideWithValue(
        AsyncValue.data(communityRole),
      ),
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
      home: ChannelDetailsPage(
        channel: channel,
        currentPubkey: keys.public,
        onMemberTap: (_, _) {},
      ),
    ),
  );
}

final _accessCard = find.byKey(const ValueKey('channel-details-access-card'));
final _defaultSetting = find.byKey(const ValueKey('channel-default-setting'));
final _joinRoleRow = find.byKey(const ValueKey('channel-join-role-row'));

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

void main() {
  testWidgets('admin sets who joins a channel and makes it a default', (
    tester,
  ) async {
    final relay = _AdmissionRelay(channelType: 'stream', visibility: 'open');
    await tester.pumpWidget(
      _app(
        relay: relay,
        channel: _channel(),
        channelRole: 'admin',
        communityRole: CommunityMemberRole.admin,
      ),
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

    await tester.tap(_joinRoleRow);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('channel-join-role-guest')));
    await tester.pumpAndSettle();
    expect(relay.changes.last, [
      ['h', _channelId],
      ['join_role', 'guest'],
    ]);
    expect(
      find.descendant(of: _joinRoleRow, matching: find.text('Guest')),
      findsOneWidget,
    );

    // An accepted change the relay's metadata does not show is never
    // reported as saved; the controls show what the relay holds.
    relay.applies = false;
    await tester.tap(
      find.descendant(of: _defaultSetting, matching: find.byType(Switch)),
    );
    await tester.pumpAndSettle();
    expect(relay.changes.last, [
      ['h', _channelId],
      ['auto_join', 'false'],
    ]);
    expect(find.textContaining('couldn’t confirm the change'), findsOneWidget);
    expect(_defaultSwitch(tester).value, isTrue);
    expect(relay.changes, hasLength(3));
  });

  testWidgets('shows only the controls each role may use', (tester) async {
    for (final (visibility, type, channelRole, communityRole, joinAs, default_)
        in const [
          // Channel admin, community member: join role only.
          ('open', 'stream', 'admin', CommunityMemberRole.member, true, false),
          // Community admin, channel member: default channel only.
          ('open', 'stream', 'member', CommunityMemberRole.owner, false, true),
          // A forum cannot be a default channel.
          ('open', 'forum', 'member', CommunityMemberRole.admin, false, false),
          // A private channel has no general access.
          (
            'private',
            'stream',
            'admin',
            CommunityMemberRole.owner,
            false,
            false,
          ),
          // Neither role manages access.
          (
            'open',
            'stream',
            'member',
            CommunityMemberRole.member,
            false,
            false,
          ),
        ]) {
      final relay = _AdmissionRelay(channelType: type, visibility: visibility);
      await tester.pumpWidget(
        _app(
          relay: relay,
          channel: _channel(visibility: visibility, type: type),
          channelRole: channelRole,
          communityRole: communityRole,
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
