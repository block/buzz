import 'dart:async';
import 'dart:convert';
import 'package:crypto/crypto.dart';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_access_provider.dart';
import 'package:buzz/features/channels/channel_access_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:buzz/shared/me/me_chat_conversion.dart';
import 'package:buzz/shared/me/me_chat_share.dart';
import 'package:buzz/features/channels/solo_conversion_sheet.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

final _keys = nostr.Keys.generate();
final _relayKeys = nostr.Keys.generate();
const _id = '11111111-1111-4111-8111-111111111111';
const _target =
    '2222222222222222222222222222222222222222222222222222222222222222';

class _Config extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://access.example', nsec: _keys.nsec);
}

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

class _Relay extends RelaySessionNotifier {
  String visibility = 'private';
  String joinRole = 'member';
  String actorRole = 'owner';
  String targetRole = 'bot';
  List<String> labels = ['buzz.solo', 'project.work'];
  int revision = 1;
  bool loseResponse = false;
  bool ignoreUpdate = false;
  bool failGroups = false;
  bool corrupt = false;
  bool corruptRoster = false;
  String targetPubkey = _target;
  List<NostrEvent> profiles = [];
  List<NostrEvent>? revokedProfiles;
  int profileReads = 0;
  NostrEvent? managedMessage;
  NostrEvent? groups;
  final published = <NostrEvent>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  NostrEvent event(int kind) => NostrEvent.fromJson(
    signNostrEvent(
      secretKey: _relayKeys.secret,
      createdAt: revision,
      kind: kind,
      tags: [
        ['d', _id],
        if (kind == 39000) ...[
          ['t', 'stream'],
          for (final label in labels) ['t', label],
          ['P', _keys.public],
          [visibility == 'private' ? 'private' : 'public'],
          ['join_role', joinRole],
          ['ttl', '604800'],
          ['name', 'Research chat'],
        ] else ...[
          if (actorRole.isNotEmpty) ['p', _keys.public, '', actorRole],
          if (targetRole.isNotEmpty) ['p', targetPubkey, '', targetRole],
        ],
      ],
      content: '',
    ).toMap(),
  );

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(30078)) return [?groups];
    if (filter.kinds.contains(0)) {
      profileReads++;
      return revokedProfiles != null && profileReads > 1
          ? revokedProfiles!
          : profiles;
    }
    if (filter.ids != null && managedMessage != null) return [managedMessage!];
    if (filter.kinds.contains(39000) == true) {
      final value = event(39000);
      return [
        corrupt
            ? NostrEvent.fromJson({...value.toJson(), 'sig': '0' * 128})
            : value,
      ];
    }
    if (filter.kinds.contains(39002) == true) {
      final value = event(39002);
      return [
        corruptRoster
            ? NostrEvent.fromJson({...value.toJson(), 'sig': '0' * 128})
            : value,
      ];
    }
    return [];
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    if (event.kind == 30078) {
      if (failGroups) {
        throw const RelayEventRejected('Could not save Solo filing');
      }
      groups = event;
      return event;
    }
    if (!ignoreUpdate) {
      visibility = event.getTagValue('visibility') ?? visibility;
      joinRole = event.getTagValue('join_role') ?? joinRole;
      if (event.tags.any((tag) => tag[0] == 't')) {
        labels = event.tags
            .where((tag) => tag[0] == 't' && tag[1].isNotEmpty)
            .map((tag) => tag[1])
            .toList();
      }
      if (event.kind == 9000) targetRole = event.getTagValue('role')!;
      if (event.kind == 9001) targetRole = '';
      revision++;
    }
    if (loseResponse) throw TimeoutException('Publication response lost');
    return event;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}

ProviderContainer _container(_Relay relay, {MeSync? sync}) => ProviderContainer(
  retry: (_, _) => null,
  overrides: [
    if (sync != null) meSyncProvider.overrideWithValue(sync),
    relayConfigProvider.overrideWith(_Config.new),
    relayAuthorProvider(
      'https://access.example',
    ).overrideWith((ref) async => _relayKeys.public),
    relaySessionProvider.overrideWith(() => relay),
    currentPubkeyProvider.overrideWithValue(_keys.public),
    channelsProvider.overrideWith(_Channels.new),
  ],
);

void main() {
  test('signed admission, viewer roles and cached models stay independent', () {
    for (final row in <(List<List<String>>, String)>[
      ([], 'member'),
      (
        [
          ['join_role', 'member'],
        ],
        'member',
      ),
      (
        [
          ['join_role', 'guest'],
        ],
        'guest',
      ),
      (
        [
          ['join_role'],
        ],
        'guest',
      ),
      (
        [
          ['join_role', 'owner'],
        ],
        'guest',
      ),
      (
        [
          ['join_role', 'member', 'extra'],
        ],
        'guest',
      ),
      (
        [
          ['join_role', 'member'],
          ['join_role', 'member'],
        ],
        'guest',
      ),
    ]) {
      final event = _Relay().event(39000);
      final parsed = NostrEvent(
        id: event.id,
        pubkey: event.pubkey,
        createdAt: event.createdAt,
        kind: event.kind,
        tags: [...event.tags.where((tag) => tag[0] != 'join_role'), ...row.$1],
        content: '',
        sig: '',
      );
      final details = channelDetailsFromEvent(parsed);
      expect(details.joinRole, row.$2);
      for (final role in [
        'owner',
        'admin',
        'member',
        'bot',
        'guest',
        'unknown',
      ]) {
        final channel = Channel(
          id: _id,
          name: '',
          channelType: 'stream',
          visibility: 'private',
          description: '',
          createdBy: _keys.public,
          createdAt: DateTime(2026),
          memberCount: 2,
          isMember: true,
          viewerRole: role,
        ).mergeDetails(details);
        final cached = Channel.fromJson(channel.toJson());
        expect(cached.joinRole, row.$2);
        expect(cached.viewerRole, role);
        expect(
          cached.canContribute,
          const {'owner', 'admin', 'member', 'bot'}.contains(role),
        );
        expect(ChannelDetails.fromChannel(cached).joinRole, row.$2);
      }
    }
  });

  testWidgets(
    'Solo access confirms disclosure, preserves roles and reconciles without resending',
    (tester) async {
      final relay = _Relay();
      final container = _container(relay);
      addTearDown(container.dispose);
      final channel = Channel(
        id: _id,
        name: 'Research chat',
        channelType: 'stream',
        labels: const ['buzz.solo'],
        visibility: 'private',
        description: '',
        createdBy: _keys.public,
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: true,
      );
      Future<void> open() async {
        await tester.pumpWidget(
          UncontrolledProviderScope(
            container: container,
            child: MaterialApp(
              theme: AppTheme.light(),
              home: Scaffold(
                body: Builder(
                  builder: (context) => TextButton(
                    onPressed: () => showChannelAccessSheet(context, channel),
                    child: const Text('Open access'),
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('Open access'));
        await tester.pumpAndSettle();
      }

      void expectSelection(String label) {
        expect(find.text('Current access: $label'), findsNothing);
        expect(find.text('Existing members keep their access.'), findsNothing);
        final selected = find.ancestor(
          of: find.text(label),
          matching: find.byType(ListTile),
        );
        expect(tester.widget<ListTile>(selected).selected, isTrue);
        expect(
          find.descendant(of: selected, matching: find.byIcon(BuzzIcons.check)),
          findsOneWidget,
        );
        expect(find.byIcon(BuzzIcons.check), findsOneWidget);
      }

      await open();
      expect(find.text('General access'), findsOneWidget);
      expect(find.text('Chat access'), findsNothing);
      expect(find.text('Channel access'), findsNothing);
      expect(find.text('Public'), findsNothing);
      expectSelection('Private');
      await tester.tap(find.text('View only'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining(
          "Everyone in this community will be able to read this chat's full history and Canvas.",
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(relay.published, isEmpty);
      expectSelection('Private');
      relay.loseResponse = true;
      await tester.tap(find.text('View only'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Change access'));
      await tester.pumpAndSettle();
      expect(relay.published.single.getTagValue('visibility'), 'open');
      expect(relay.published.single.getTagValue('join_role'), 'guest');
      expect(relay.actorRole, 'owner');
      expect(relay.targetRole, 'bot');
      expect(find.text('Check save status'), findsOneWidget);
      container.invalidate(relayAuthorProvider('https://access.example'));
      await tester.pumpAndSettle();
      expect(
        container.read(channelAccessProvider).containsKey(_id),
        isTrue,
        reason: 'Discovery refresh must not discard an uncertain write',
      );
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open access'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Check save status'));
      await tester.pumpAndSettle();
      expect(relay.published, hasLength(1));
      expect(find.text('Check save status'), findsNothing);
      expectSelection('View only');
      relay.loseResponse = false;
      await tester.tap(find.text('Private'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Change access'));
      await tester.pumpAndSettle();
      expect(relay.visibility, 'private');
      expect(relay.joinRole, 'guest');
      expect(relay.published.last.getTagValue('join_role'), isNull);
      expect(relay.labels, ['buzz.solo', 'project.work']);
      expectSelection('Private');
      // A signed request whose response is lost may never reach the relay.
      relay.loseResponse = true;
      relay.ignoreUpdate = true;
      await tester.tap(find.text('View only'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Change access'));
      await tester.pumpAndSettle();
      final writesBeforeDiscard = relay.published.length;
      final discardedAttempt = container.read(channelAccessProvider)[_id]!;
      expect(find.text('Discard and reload'), findsOneWidget);
      await tester.tap(find.text('Check save status'));
      await tester.pumpAndSettle();
      expect(find.text('Check save status'), findsOneWidget);
      await tester.tap(find.text('Discard and reload'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('The earlier request may still arrive.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('Check save status'), findsOneWidget);
      await tester.tap(find.text('Discard and reload'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Discard and reload'));
      await tester.pumpAndSettle();
      expect(relay.published, hasLength(writesBeforeDiscard));
      expect(find.text('Check save status'), findsNothing);
      expectSelection('Private');

      // Another administrator can replace a landed request before readback.
      relay.ignoreUpdate = false;
      await tester.tap(find.text('View only'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Change access'));
      await tester.pumpAndSettle();
      final replacementAttempt = container.read(channelAccessProvider)[_id]!;
      await expectLater(
        container
            .read(channelAccessProvider.notifier)
            .discardSave(_id, discardedAttempt),
        throwsStateError,
      );
      expect(
        container.read(channelAccessProvider)[_id],
        same(replacementAttempt),
      );
      relay.visibility = 'private';
      relay.revision++;
      await tester.tap(find.text('Check save status'));
      await tester.pumpAndSettle();
      expect(
        find.textContaining(
          'Access changed elsewhere; review the current setting.',
        ),
        findsOneWidget,
      );
      expect(find.text('Check save status'), findsNothing);
      expectSelection('Private');
      relay.loseResponse = false;
      await tester.tap(find.text('View only'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Change access'));
      await tester.pumpAndSettle();
      expectSelection('View only');
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
      relay.visibility = 'open';
      relay.joinRole = 'member';
      relay.revision++;
      await tester.tap(find.text('Open access'));
      await tester.pumpAndSettle();
      expect(find.text('Current access: Public (existing)'), findsOneWidget);
      expect(find.text('Public'), findsNothing);
      expect(find.byIcon(BuzzIcons.check), findsNothing);
      await tester.tap(find.text('View only'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Change access'));
      await tester.pumpAndSettle();
      expect(relay.visibility, 'open');
      expect(relay.joinRole, 'guest');
      expectSelection('View only');
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
    },
  );

  for (final scenario in [
    (
      name: 'View only to Private with a Guest',
      actorRole: 'owner',
      solo: true,
      visibility: 'open',
      joinRole: 'guest',
      targetRole: 'guest',
      choice: 'Private',
      expected:
          'Only chat members will have access. People who joined while it was View only are members and keep reading until you remove them in People.',
    ),
    (
      name: 'View only to Private with only contributors',
      actorRole: 'owner',
      solo: true,
      visibility: 'open',
      joinRole: 'guest',
      targetRole: 'bot',
      choice: 'Private',
      expected: 'Only chat members will have access.',
    ),
    (
      name: 'Public existing to View only',
      actorRole: 'owner',
      solo: true,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'bot',
      choice: 'View only',
      expected:
          "Everyone in this community will be able to read this chat's full history and Canvas. People already in the chat keep their access.",
    ),
    (
      name: 'Private to View only with a Guest',
      actorRole: 'owner',
      solo: true,
      visibility: 'private',
      joinRole: 'guest',
      targetRole: 'guest',
      choice: 'View only',
      expected:
          "Everyone in this community will be able to read this chat's full history and Canvas.",
    ),
    (
      name: 'Public existing to Private with a Guest',
      actorRole: 'owner',
      solo: true,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'guest',
      choice: 'Private',
      expected: 'Only chat members will have access.',
    ),
    (
      name: 'ordinary View only to Private with a Guest',
      actorRole: 'owner',
      solo: false,
      visibility: 'open',
      joinRole: 'guest',
      targetRole: 'guest',
      choice: 'Private',
      expected:
          'Only people already added to this channel will be able to read. Existing members keep their access.',
    ),
    (
      name: 'participating member inspection',
      actorRole: 'member',
      solo: false,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'bot',
      choice: null,
      expected: 'Only owners and admins can change access.',
    ),
    (
      name: 'participating bot inspection',
      actorRole: 'bot',
      solo: false,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'bot',
      choice: null,
      expected: 'Only owners and admins can change access.',
    ),
    (
      name: 'Private member inspection',
      actorRole: 'member',
      solo: false,
      visibility: 'private',
      joinRole: 'member',
      targetRole: 'bot',
      choice: null,
      expected: 'Only owners and admins can change access.',
    ),
    (
      name: 'Guest inspection',
      actorRole: 'guest',
      solo: false,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'bot',
      choice: null,
      expected:
          'Viewing only. Ask an owner or admin to participate in this channel.',
    ),
    (
      name: 'Public nonmember inspection',
      actorRole: '',
      solo: false,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'bot',
      choice: null,
      expected: 'Join this channel to participate.',
    ),
    (
      name: 'View only nonmember inspection',
      actorRole: '',
      solo: false,
      visibility: 'open',
      joinRole: 'guest',
      targetRole: 'bot',
      choice: null,
      expected:
          'Viewing only. Ask an owner or admin to participate in this channel.',
    ),
    (
      name: 'Solo Guest inspection',
      actorRole: 'guest',
      solo: true,
      visibility: 'open',
      joinRole: 'guest',
      targetRole: 'bot',
      choice: null,
      expected:
          'Viewing only. Ask an owner or admin to participate in this chat.',
    ),
    (
      name: 'Solo nonmember inspection',
      actorRole: '',
      solo: true,
      visibility: 'open',
      joinRole: 'member',
      targetRole: 'bot',
      choice: null,
      expected:
          'Viewing only. Ask an owner or admin to participate in this chat.',
    ),
  ]) {
    testWidgets('access confirmation: ${scenario.name}', (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final relay = _Relay()
        ..actorRole = scenario.actorRole
        ..visibility = scenario.visibility
        ..joinRole = scenario.joinRole
        ..targetRole = scenario.targetRole
        ..labels = scenario.solo ? ['buzz.solo'] : [];
      final container = _container(relay);
      addTearDown(container.dispose);
      final channel = Channel(
        id: _id,
        name: 'Research chat',
        channelType: 'stream',
        visibility: scenario.visibility,
        joinRole: scenario.joinRole,
        labels: relay.labels,
        description: '',
        createdBy: _keys.public,
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: scenario.actorRole.isNotEmpty,
        viewerRole: scenario.actorRole.isEmpty ? null : scenario.actorRole,
      );
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp(
            theme: AppTheme.light(),
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(
                context,
              ).copyWith(textScaler: TextScaler.linear(1.5)),
              child: child!,
            ),
            home: Scaffold(
              body: Builder(
                builder: (context) => TextButton(
                  onPressed: () => showChannelAccessSheet(context, channel),
                  child: const Text('Open access'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open access'));
      await tester.pumpAndSettle();
      expect(find.text('General access'), findsOneWidget);
      final choice = scenario.choice;
      if (choice == null) {
        final semantics = tester.ensureSemantics();
        expect(find.text(scenario.expected), findsOneWidget);
        expect(find.textContaining('Current access:'), findsNothing);
        final selectedLabel = scenario.visibility == 'private'
            ? 'Private'
            : scenario.joinRole == 'guest'
            ? 'View only'
            : scenario.solo
            ? 'Public (existing)'
            : 'Public';
        final options = find.byType(ListTile);
        expect(
          options,
          findsNWidgets(
            scenario.solo && selectedLabel != 'Public (existing)' ? 2 : 3,
          ),
        );
        for (final option in options.evaluate()) {
          final tile = option.widget as ListTile;
          expect(tile.enabled, isFalse);
          expect(tile.selected, (tile.title! as Text).data == selectedLabel);
          final label = (tile.title! as Text).data!;
          final finder = find.widgetWithText(ListTile, label);
          await tester.ensureVisible(finder);
          await tester.pumpAndSettle();
          expect(
            tester.getSemantics(finder),
            matchesSemantics(
              isButton: true,
              isSelected: label == selectedLabel,
              hasEnabledState: true,
              isEnabled: false,
              hasSelectedState: true,
              label: '$label\n${(tile.subtitle! as Text).data}',
            ),
          );
          await tester.tap(finder);
          await tester.pumpAndSettle();
          expect(find.byType(AlertDialog), findsNothing);
        }
        expect(find.byIcon(BuzzIcons.check), findsOneWidget);
        expect(find.text('Change access'), findsNothing);
        expect(relay.published, isEmpty);
        semantics.dispose();
        return;
      }
      await tester.ensureVisible(find.text(choice));
      await tester.pumpAndSettle();
      await tester.tap(find.text(choice));
      await tester.pumpAndSettle();
      final dialog = tester.widget<AlertDialog>(find.byType(AlertDialog));
      expect((dialog.content! as Text).data, scenario.expected);
      expect(relay.published, isEmpty);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(relay.published, isEmpty);
      expect(find.byType(AlertDialog), findsNothing);
    });
  }

  testWidgets(
    'an unverified Guest roster cannot produce an access confirmation',
    (tester) async {
      final relay = _Relay()
        ..visibility = 'open'
        ..joinRole = 'guest'
        ..targetRole = 'guest'
        ..corruptRoster = true;
      final container = _container(relay);
      addTearDown(container.dispose);
      final channel = Channel(
        id: _id,
        name: 'Research chat',
        channelType: 'stream',
        labels: const ['buzz.solo'],
        visibility: 'open',
        joinRole: 'guest',
        description: '',
        createdBy: _keys.public,
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: true,
      );
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(body: ChannelAccessSheet(channel: channel)),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        find.text('Current access could not be confirmed.'),
        findsOneWidget,
      );
      expect(find.text('Private'), findsNothing);
      expect(find.byType(AlertDialog), findsNothing);
      expect(relay.published, isEmpty);
    },
  );

  test(
    'fresh access rejects stale consent and protects roles; conversion preserves open Guest policy',
    () async {
      final relay = _Relay()
        ..visibility = 'open'
        ..joinRole = 'guest';
      final container = _container(relay);
      addTearDown(container.dispose);
      final controller = container.read(channelAccessProvider.notifier);
      var base = await controller.load(_id);
      await expectLater(
        controller.setAccess(_id, base, 'public'),
        throwsStateError,
      );
      expect(relay.published, isEmpty);
      relay.actorRole = 'guest';
      relay.revision++;
      await expectLater(
        controller.setAccess(_id, base, 'private'),
        throwsStateError,
      );
      await expectLater(controller.requireContribution(_id), throwsStateError);
      expect(relay.published, isEmpty);
      relay.actorRole = 'owner';
      relay.revision++;
      base = await controller.load(_id);
      await expectLater(
        controller.changeMember(_id, base, _keys.public, role: 'guest'),
        throwsStateError,
      );
      await expectLater(
        controller.changeMember(_id, base, _target, role: 'guest'),
        throwsStateError,
      );
      relay.labels = ['project.work'];
      relay.revision++;
      base = await controller.load(_id);
      await controller.changeMember(_id, base, _target, role: 'guest');
      expect(relay.targetRole, 'guest');
      expect(relay.joinRole, 'guest');
      relay.labels = ['buzz.solo', 'project.work'];
      relay.revision++;
      base = await controller.load(_id);
      await controller.convert(_id, base);
      expect(relay.labels, ['project.work']);
      expect(relay.visibility, 'open');
      expect(relay.joinRole, 'guest');
      expect(relay.published.last.tags, [
        ['h', _id],
        ['t', 'project.work'],
      ]);
    },
  );
  testWidgets(
    'conversion preselects View only and recovers filing after the metadata is saved',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = _Relay()
        ..visibility = 'open'
        ..joinRole = 'guest'
        ..failGroups = true;
      final sync = MeSync(
        relay,
        RelayConfig(baseUrl: 'https://access.example', nsec: _keys.nsec),
        prefs: prefs,
      );
      relay.groups = NostrEvent.fromJson(
        signNostrEvent(
          kind: 30078,
          secretKey: _keys.secret,
          content: nip44Encrypt(
            sync.key,
            const MeGroups(channels: [_id]).encode(sync.community),
          ),
          tags: [
            ['d', MeGroups.coordinate(sync.community)],
            ['t', MeGroups.tag],
          ],
          createdAt: 1,
        ).toMap(),
      );
      // The shared service owns persistence; only relay and disk are test boundaries.
      final container = _container(relay, sync: sync);
      addTearDown(container.dispose);
      final channel = Channel(
        id: _id,
        name: 'Research chat',
        channelType: 'stream',
        labels: const ['buzz.solo', 'project.work'],
        visibility: 'open',
        joinRole: 'guest',
        description: '',
        createdBy: _keys.public,
        createdAt: DateTime(2026),
        memberCount: 2,
        isMember: true,
      );
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp(
            theme: AppTheme.light(),
            home: Scaffold(body: SoloConversionSheet(channel: channel)),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<DropdownButtonFormField<String>>(
              find.byType(DropdownButtonFormField<String>),
            )
            .initialValue,
        'view-only',
      );
      await tester.tap(find.text('Convert to channel'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Convert'));
      await tester.pumpAndSettle();
      expect(relay.labels, ['project.work']);
      expect(relay.visibility, 'open');
      expect(relay.joinRole, 'guest');
      expect(MeChatConversion.pending(sync, _id), isNotNull);
      expect(
        relay.published.where((event) => event.kind == 9002),
        hasLength(1),
      );
      relay.failGroups = false;
      await tester.tap(find.text('Finish conversion'));
      await tester.pumpAndSettle();
      expect(MeChatConversion.pending(sync, _id), isNull);
      expect(
        relay.published.where((event) => event.kind == 9002),
        hasLength(1),
      );
    },
  );

  test('each conversion choice saves its exact admission policy', () async {
    final relay = _Relay();
    final container = _container(relay);
    addTearDown(container.dispose);
    final controller = container.read(channelAccessProvider.notifier);
    for (final choice in ['private', 'view-only', 'public']) {
      relay.labels = ['buzz.solo', 'project.work'];
      relay.visibility = 'open';
      relay.joinRole = 'guest';
      relay.revision++;
      await controller.convert(_id, await controller.load(_id), access: choice);
      expect(relay.visibility, choice == 'private' ? 'private' : 'open');
      expect(relay.joinRole, choice == 'public' ? 'member' : 'guest');
      expect(relay.labels, ['project.work']);
      expect(relay.published.last.getTagValue('ttl'), isNull);
    }
  });

  test(
    'a share preserves the frozen audience and reuses one signed message after a lost response',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final sync = MeSync(
        _Relay(),
        RelayConfig(baseUrl: 'https://access.example', nsec: _keys.nsec),
        prefs: prefs,
      );
      final intent = MeChatShare.create(
        sync,
        source: _id,
        name: 'Research [chat]',
        destination: 'destination',
        destinationName: 'team',
        audience: {_target: 'bot'},
      );
      final messages = <String>[];
      var lose = true;
      final members = <String, String>{};
      Future<void> attempt(MeChatShare value) => value.complete(
        sync,
        ensureRecipient: (key, role) async {
          members.putIfAbsent(key, () => role);
        },
        send: (event) async {
          messages.add(event.id);
          if (lose) throw TimeoutException('Lost response');
        },
      );
      await expectLater(attempt(intent), throwsA(isA<TimeoutException>()));
      final restored = MeChatShare.pending(sync, _id)!;
      expect(restored.audience, {_target: 'bot'});
      expect(restored.message.id, intent.message.id);
      lose = false;
      await attempt(restored);
      expect(messages, [intent.message.id, intent.message.id]);
      expect(members, {_target: 'bot'});
      expect(MeChatShare.pending(sync, _id), isNull);
    },
  );
  test(
    'access rejects forged metadata and rechecks a Guest owner’s agent removal',
    () async {
      final agent = nostr.Keys.generate();
      final relay = _Relay()
        ..labels = []
        ..actorRole = 'guest'
        ..targetPubkey = agent.public;
      final container = _container(relay);
      addTearDown(container.dispose);
      final controller = container.read(channelAccessProvider.notifier);
      relay.corrupt = true;
      await expectLater(
        controller.load(_id),
        throwsA(isA<nostr.EventValidationException>()),
      );
      relay.corrupt = false;
      relay.actorRole = 'unknown';
      relay.visibility = 'open';
      await expectLater(
        controller.addMember(_id, _target, 'member'),
        throwsStateError,
      );
      expect(
        relay.published,
        isEmpty,
        reason: 'An unknown existing role is not a nonmember preview',
      );
      relay.actorRole = 'guest';
      final digest = sha256
          .convert(utf8.encode('nostr:agent-auth:${agent.public}:'))
          .toString();
      final profile = NostrEvent.fromJson(
        signNostrEvent(
          kind: 0,
          secretKey: agent.secret,
          content: '{}',
          createdAt: 1,
          tags: [
            [
              'auth',
              _keys.public,
              '',
              nostr.Schnorr.sign(secretKey: _keys.secret, message: digest),
            ],
          ],
        ).toMap(),
      );
      final revoked = NostrEvent.fromJson(
        signNostrEvent(
          kind: 0,
          secretKey: agent.secret,
          content: '{}',
          createdAt: 2,
        ).toMap(),
      );
      relay.profiles = [profile];
      relay.revokedProfiles = [revoked];
      var base = await controller.load(_id);
      await expectLater(
        controller.changeMember(_id, base, agent.public),
        throwsStateError,
      );
      expect(relay.published, isEmpty);
      relay.revokedProfiles = null;
      relay.profileReads = 0;
      relay.managedMessage = NostrEvent.fromJson(
        signNostrEvent(
          kind: EventKind.streamMessage,
          secretKey: agent.secret,
          content: 'Agent result',
          tags: [
            ['h', _id],
          ],
        ).toMap(),
      );
      await controller.requireMessageManagement(_id, relay.managedMessage!.id);
      relay.profiles = [revoked];
      await expectLater(
        controller.requireMessageManagement(_id, relay.managedMessage!.id),
        throwsStateError,
      );
      relay.profiles = [profile];
      relay.labels = ['buzz.solo'];
      relay.revision++;
      base = await controller.load(_id);
      await expectLater(
        controller.changeMember(_id, base, agent.public),
        throwsStateError,
      );
      relay.labels = [];
      relay.revision++;
      base = await controller.load(_id);
      await controller.changeMember(_id, base, agent.public);
      expect(relay.targetRole, '');
      expect(relay.published.single.kind, 9001);
    },
  );
}
