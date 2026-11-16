import 'dart:async';
import 'dart:ui' as ui;

import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/me/me_page.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_assistants.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart' show CustomSemanticsAction;
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

class _Config extends RelayConfigNotifier {
  _Config(this.config);
  final RelayConfig config;
  @override
  RelayConfig build() => config;
}

// Only the relay and preference storage are faked in the archive path. The
// real signer, ChannelActions and ChannelsNotifier submit and reconcile it.
class _ArchiveRelay extends RelaySessionNotifier {
  _ArchiveRelay(this.owner);
  final String owner;
  final submitted = <NostrEvent>[];
  final archived = <String>{'archived'};
  Completer<void>? acknowledgement;

  static const chats = {
    'grouped': 'Project chat',
    'loose': 'Loose chat',
    'legacy': 'Legacy chat',
    'ordinary': 'General',
    'archived': 'Old chat',
  };

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  List<NostrEvent> events(NostrFilter filter) => [
    if (filter.kinds.contains(39002) && filter.until == null)
      for (final id in chats.keys)
        NostrEvent(
          id: 'members-$id',
          pubkey: 'relay',
          createdAt: 1,
          kind: 39002,
          tags: [
            ['d', id],
            ['p', owner, '', 'owner'],
          ],
          content: '',
          sig: '',
        ),
    if (filter.kinds.contains(39000))
      for (final entry in chats.entries)
        if (filter.tags['#d']?.contains(entry.key) == true)
          NostrEvent(
            id: 'metadata-${entry.key}-${archived.contains(entry.key)}',
            pubkey: 'relay',
            createdAt: archived.contains(entry.key) ? 2 : 1,
            kind: 39000,
            tags: [
              ['d', entry.key],
              ['name', entry.value],
              ['t', 'stream'],
              ['private'],
              ['P', owner],
              if (entry.key != 'legacy' && entry.key != 'ordinary')
                ['t', 'buzz.solo'],
              if (archived.contains(entry.key)) ['archived', 'true'],
            ],
            content: '',
            sig: '',
          ),
  ];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => events(filter);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [for (final filter in filters) ...events(filter)];

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

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    submitted.add(event);
    await acknowledgement?.future;
    archived.add(event.getTagValue('h')!);
    return event;
  }
}

void main() {
  for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
    testWidgets('Solo archive journey on $platform', (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final owner = nostr.Keys.generate();
      final relay = _ArchiveRelay(owner.public);
      final embedded = platform == TargetPlatform.android;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(
              () => _Config(
                RelayConfig(baseUrl: 'https://relay.test', nsec: owner.nsec),
              ),
            ),
            relaySessionProvider.overrideWith(() => relay),
            defaultAgentProvider.overrideWith((ref) => Stream.value(null)),
            meAssistantsProvider.overrideWith((ref) async => []),
            meGroupsProvider.overrideWith(
              (ref) => Stream.value(
                const MeGroups(
                  groups: [
                    {'id': 'project', 'name': 'App work'},
                  ],
                  assignments: {'grouped': 'project'},
                  channels: ['legacy'],
                ),
              ),
            ),
          ],
          child: MaterialApp(
            theme: AppTheme.light().copyWith(platform: platform),
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(
                disableAnimations: true,
                textScaler: const TextScaler.linear(1.5),
              ),
              child: child!,
            ),
            home: embedded
                ? const Scaffold(
                    body: CustomScrollView(slivers: [MePage(sliver: true)]),
                  )
                : const MePage(),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final context = tester.element(find.byType(MePage));
      final container = ProviderScope.containerOf(context);
      expect(find.byType(Dismissible), findsNWidgets(3));
      expect(find.text('General'), findsNothing);
      expect(find.text('Old chat'), findsNothing);

      // Scrolling, a short left drag, and a right drag never archive.
      for (final offset in [
        const Offset(0, -60),
        const Offset(-30, 0),
        const Offset(500, 0),
      ]) {
        await tester.drag(find.text('Project chat'), offset);
        await tester.pumpAndSettle();
        expect(relay.submitted, isEmpty);
        expect(find.byType(Dismissible), findsNWidgets(3));
      }

      // Keep a write in flight, reject it (including permission/offline
      // failures), then retry the same gesture through the real action.
      relay.acknowledgement = Completer<void>();
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(relay.submitted, hasLength(1));
      expect(relay.submitted.single.kind, 9002);
      expect(relay.submitted.single.tags, [
        ['h', 'grouped'],
        ['archived', 'true'],
      ]);
      expect(relay.submitted.single.sig, isNotEmpty);
      expect(relay.archived, isNot(contains('grouped')));
      expect(find.text('Archiving…').hitTestable(), findsOneWidget);
      await tester.drag(
        find.byKey(const ValueKey('archive-solo-grouped')),
        const Offset(-700, 0),
      );
      await tester.pumpAndSettle();
      expect(relay.submitted, hasLength(1));
      relay.acknowledgement!.completeError(StateError('Relay unavailable'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Couldn’t archive chat:'), findsOneWidget);
      expect(find.text('Project chat').hitTestable(), findsOneWidget);
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .firstWhere((channel) => channel.id == 'grouped')
            .isArchived,
        isFalse,
      );

      relay.acknowledgement = null;
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      expect(find.text('Project chat'), findsNothing);
      expect(find.text('Loose chat'), findsOneWidget);
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .firstWhere((channel) => channel.id == 'grouped')
            .isArchived,
        isTrue,
      );

      // The same action is available without a drag, on exactly one labelled
      // semantics node, and legacy Solo placement gets it too.
      final node = tester.getSemantics(find.bySemanticsLabel('Legacy chat'));
      final ids = node.getSemanticsData().customSemanticsActionIds!;
      final archive = ids.singleWhere(
        (id) => CustomSemanticsAction.getAction(id)!.label == 'Archive chat',
      );
      node.owner!.performAction(
        node.id,
        ui.SemanticsAction.customAction,
        archive,
      );
      await tester.pumpAndSettle();
      expect(find.text('Legacy chat'), findsNothing);
      expect(relay.archived, containsAll(['grouped', 'legacy']));
      expect(find.text('Loose chat').hitTestable(), findsOneWidget);
      expect(tester.takeException(), isNull);
    }, variant: TargetPlatformVariant.only(platform));
  }
}
