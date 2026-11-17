import 'dart:async';
import 'dart:ui' as ui;

import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
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
  bool failRefresh = false;
  int failedRefreshes = 0;
  VoidCallback? afterAcceptance;

  static const chats = {
    'grouped': 'Project chat',
    'loose': 'Loose chat',
    'legacy': 'Legacy chat',
    'ordinary': 'General',
    'archived': 'Old chat',
    'shared': 'Shared chat',
  };

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  List<NostrEvent> events(NostrFilter filter) {
    if (failRefresh && filter.kinds.contains(39002)) {
      failedRefreshes++;
      throw StateError('List unavailable');
    }
    return [
      if (filter.kinds.contains(39002) && filter.until == null)
        for (final id in chats.keys)
          NostrEvent(
            id: 'members-$id',
            pubkey: 'relay',
            createdAt: 1,
            kind: 39002,
            tags: [
              ['d', id],
              ['p', owner, '', id == 'shared' ? 'member' : 'owner'],
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
                ['P', entry.key == 'shared' ? 'someone-else' : owner],
                if (entry.key != 'legacy' && entry.key != 'ordinary')
                  ['t', 'buzz.solo'],
                if (archived.contains(entry.key)) ['archived', 'true'],
              ],
              content: '',
              sig: '',
            ),
    ];
  }

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
    bool resumeSince = true,
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
    afterAcceptance?.call();
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
      final pane = ConversationPaneController();
      addTearDown(pane.dispose);
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
            home: ConversationPaneScope(
              controller: pane,
              child: embedded
                  ? const Scaffold(
                      body: CustomScrollView(slivers: [MePage(sliver: true)]),
                    )
                  : const MePage(),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final context = tester.element(find.byType(MePage));
      final container = ProviderScope.containerOf(context);
      final shared = find.text('Shared chat');
      expect(shared, findsOneWidget);
      expect(
        find.ancestor(of: shared, matching: find.byType(Dismissible)),
        findsNothing,
      );
      final sharedActions = tester
          .getSemantics(find.bySemanticsLabel('Shared chat'))
          .getSemanticsData()
          .customSemanticsActionIds;
      expect(
        (sharedActions ?? const <int>[]).map(
          (id) => CustomSemanticsAction.getAction(id)!.label,
        ),
        isNot(contains('Archive chat')),
      );
      await tester.drag(shared, const Offset(-700, 0));
      await tester.pumpAndSettle();
      expect(relay.submitted, isEmpty);
      expect(shared.hitTestable(), findsOneWidget);
      await tester.tap(shared);
      await tester.pumpAndSettle();
      expect(pane.selectedId, 'shared');
      expect(pane.detailBuilder!(context), isA<ChannelDetailPage>());
      pane.clear();
      await tester.pumpAndSettle();
      expect(shared.hitTestable(), findsOneWidget);
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

      // A full swipe only offers the existing lifecycle confirmation.
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      expect(find.text('Archive #Project chat?'), findsOneWidget);
      expect(find.text('The channel will become read-only.'), findsOneWidget);
      expect(relay.submitted, isEmpty);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(relay.submitted, isEmpty);
      expect(find.text('Project chat').hitTestable(), findsOneWidget);

      // Keep a write in flight, reject it (including permission/offline
      // failures), then retry the same gesture through the real action.
      relay.acknowledgement = Completer<void>();
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(relay.submitted, isEmpty);
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
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
      final pendingActions = tester
          .getSemantics(find.bySemanticsLabel('Project chat, archiving'))
          .getSemanticsData()
          .customSemanticsActionIds;
      expect(
        (pendingActions ?? const <int>[]).map(
          (id) => CustomSemanticsAction.getAction(id)!.label,
        ),
        isNot(contains('Archive chat')),
      );
      // Collapse fully disposes the row, then expand before acknowledgement.
      await tester.tap(find.text('App work'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('archive-solo-grouped')), findsNothing);
      await tester.tap(find.text('App work'));
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Project chat, archiving'), findsOneWidget);
      expect(
        tester
            .getSemantics(find.bySemanticsLabel('Project chat, archiving'))
            .getSemanticsData()
            .customSemanticsActionIds,
        isEmpty,
      );
      await tester.drag(
        find.byKey(const ValueKey('archive-solo-grouped')),
        const Offset(-700, 0),
      );
      await tester.pumpAndSettle();
      expect(relay.submitted, hasLength(1));
      await tester.tap(find.text('App work'));
      await tester.pumpAndSettle();
      expect(find.text('Project chat'), findsNothing);
      relay.acknowledgement!.completeError(StateError('Relay unavailable'));
      await tester.pumpAndSettle();
      expect(find.text('Couldn’t archive chat. Try again.'), findsOneWidget);
      await tester.tap(find.text('App work'));
      await tester.pumpAndSettle();
      expect(find.text('Project chat').hitTestable(), findsOneWidget);
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .firstWhere((channel) => channel.id == 'grouped')
            .isArchived,
        isFalse,
      );

      relay.acknowledgement = Completer<void>();
      pane.open('grouped', (_) => const SizedBox());
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await tester.pumpAndSettle();
      // Metadata can arrive before the write acknowledgement. That is still
      // reconciled, and must not leave an accepted ID behind after unarchive.
      relay.archived.add('grouped');
      await container.read(channelsProvider.notifier).refresh();
      await tester.pumpAndSettle();
      expect(pane.selectedId, 'grouped');
      relay.acknowledgement!.complete();
      await tester.pumpAndSettle();
      relay.acknowledgement = null;
      expect(find.text('Project chat'), findsNothing);
      expect(pane.selectedId, isNull);
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
      expect(find.text('Archive #Legacy chat?'), findsOneWidget);
      expect(relay.submitted, hasLength(2));
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(relay.submitted, hasLength(2));
      expect(find.text('Legacy chat'), findsOneWidget);
      node.owner!.performAction(
        node.id,
        ui.SemanticsAction.customAction,
        archive,
      );
      await tester.pumpAndSettle();
      expect(find.text('Archive #Legacy chat?'), findsOneWidget);
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await tester.pumpAndSettle();
      expect(find.text('Legacy chat'), findsNothing);
      expect(relay.archived, containsAll(['grouped', 'legacy']));
      expect(find.text('Loose chat').hitTestable(), findsOneWidget);
      ScaffoldMessenger.of(context).clearSnackBars();
      await tester.pumpAndSettle();
      // Relay acceptance plus a real list-read failure retains cached rows.
      // It must retain accepted knowledge too: no second kind:9002, including
      // after the section disposes/remounts. Recovery retries only the read.
      relay.failRefresh = true;
      pane.open('shared', (_) => const SizedBox());
      await tester.drag(find.text('Loose chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await tester.pumpAndSettle();
      expect(relay.failedRefreshes, greaterThan(0));
      expect(relay.archived, contains('loose'));
      expect(relay.submitted, hasLength(4));
      expect(find.text('Couldn’t archive chat. Try again.'), findsNothing);
      expect(
        find.text('Chat archived. Refresh to update the list.'),
        findsOneWidget,
      );
      expect(pane.selectedId, 'shared');
      await tester.tap(find.text('Chats'));
      await tester.pumpAndSettle();
      expect(find.text('Loose chat'), findsNothing);
      await tester.tap(find.text('Chats'));
      await tester.pumpAndSettle();
      final acceptedNode = tester.getSemantics(
        find.bySemanticsLabel('Loose chat, archived, awaiting refresh'),
      );
      final acceptedActions = acceptedNode
          .getSemanticsData()
          .customSemanticsActionIds!;
      expect(
        acceptedActions.map((id) => CustomSemanticsAction.getAction(id)!.label),
        ['Refresh chat status'],
      );
      await tester.drag(find.text('Loose chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      expect(find.text('Archive #Loose chat?'), findsNothing);
      expect(relay.submitted, hasLength(4));
      relay.failRefresh = false;
      acceptedNode.owner!.performAction(
        acceptedNode.id,
        ui.SemanticsAction.customAction,
        acceptedActions.single,
      );
      await tester.pumpAndSettle();
      expect(find.text('Loose chat'), findsNothing);
      expect(relay.submitted, hasLength(4));

      // Once reconciliation has observed the archive, a genuine later
      // unarchive is actionable again; completed IDs do not accumulate.
      relay.archived.remove('grouped');
      await container.read(channelsProvider.notifier).refresh();
      await tester.pumpAndSettle();
      expect(find.bySemanticsLabel('Project chat'), findsOneWidget);
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      expect(find.text('Archive #Project chat?'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(relay.submitted, hasLength(4));

      // An acknowledgement from a retired community cannot mutate the new
      // scope's row state, feedback, or same-ID tablet selection.
      relay.acknowledgement = Completer<void>();
      await tester.drag(find.text('Project chat'), const Offset(-700, 0));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await tester.pumpAndSettle();
      expect(relay.submitted, hasLength(5));
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://other-relay.test', nsec: owner.nsec);
      await tester.pumpAndSettle();
      pane.open('grouped', (_) => const SizedBox());
      relay.acknowledgement!.complete();
      await tester.pumpAndSettle();
      expect(pane.selectedId, 'grouped');
      expect(find.bySemanticsLabel('Project chat'), findsOneWidget);
      expect(
        find.bySemanticsLabel('Project chat, archived, awaiting refresh'),
        findsNothing,
      );
      expect(find.text('Couldn’t archive chat. Try again.'), findsNothing);
      expect(tester.takeException(), isNull);
    }, variant: TargetPlatformVariant.only(platform));
  }
  test(
    'archive acceptance is reported before reconciliation errors propagate',
    () async {
      final owner = nostr.Keys.generate();
      final relay = _ArchiveRelay(owner.public);
      var valid = true;
      var accepted = 0;
      relay.afterAcceptance = () => valid = false;
      final actionsProvider = Provider<ChannelActions>(
        (ref) => ChannelActions(
          ref: ref,
          session: relay,
          signedEventRelay: SignedEventRelay(session: relay, nsec: owner.nsec),
          currentPubkey: owner.public,
          isCommunityValid: () => valid,
        ),
      );
      final container = ProviderContainer();
      addTearDown(container.dispose);
      await expectLater(
        container
            .read(actionsProvider)
            .archiveChannel('grouped', onAccepted: () => accepted++),
        throwsA(isA<StateError>()),
      );
      expect(accepted, 1);
      expect(relay.submitted, hasLength(1));
      expect(relay.archived, contains('grouped'));

      valid = true;
      relay.acknowledgement = Completer<void>();
      final rejected = container
          .read(actionsProvider)
          .archiveChannel('loose', onAccepted: () => accepted++);
      final rejection = expectLater(rejected, throwsA(isA<StateError>()));
      relay.acknowledgement!.completeError(StateError('Denied'));
      await rejection;
      expect(accepted, 1);
      expect(relay.archived, isNot(contains('loose')));
    },
  );
}
