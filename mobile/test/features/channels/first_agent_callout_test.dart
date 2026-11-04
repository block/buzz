import 'dart:async';
import 'dart:ui' show SemanticsAction;

import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/channels/first_agent_callout.dart';
import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/me/me_sync_test.dart' show FakeRelay;

const _card = 'Create your first agent';
const _delay = Duration(seconds: 3);

void main() {
  final nsec = nostr.Nip19.encode(
    prefix: nostr.Nip19Prefix.nsec,
    data: '0' * 63 + '1',
  );
  late SharedPreferences prefs;
  late StreamController<String?> bestie;
  late int choices;
  late int otherTaps;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
    bestie = StreamController<String?>.broadcast();
    choices = 0;
    otherTaps = 0;
  });
  tearDown(() => bestie.close());

  Future<void> pumpHome(
    WidgetTester tester, {
    String community = 'https://a.test',
    bool remoteAgents = true,
    List<OwnedAgentDirectoryEntry> owned = const [],
    String? initialBestie,
    bool reducedMotion = false,
    bool waitForCard = true,
  }) async {
    final relay = FakeRelay();
    final scope = MeSync(
      relay,
      RelayConfig(baseUrl: community, nsec: nsec),
      prefs: prefs,
    );
    await tester.pumpWidget(
      ProviderScope(
        // A new key per call rebuilds the providers, like a fresh launch.
        key: UniqueKey(),
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          meSyncProvider.overrideWithValue(scope),
          relaySessionProvider.overrideWith(() => relay),
          remoteAgentsEnabledProvider.overrideWithValue(remoteAgents),
          ownedAgentDirectoryProvider.overrideWith((ref) async => owned),
          defaultAgentProvider.overrideWith((ref) async* {
            yield initialBestie;
            yield* bestie.stream;
          }),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(disableAnimations: reducedMotion),
            child: child!,
          ),
          home: Scaffold(
            body: Center(
              child: TextButton(
                onPressed: () => otherTaps++,
                child: const Text('Channel'),
              ),
            ),
            bottomNavigationBar: Align(
              alignment: Alignment.centerRight,
              heightFactor: 1,
              child: DefaultAgentLauncher(onChooseAgent: () => choices++),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    if (waitForCard) {
      await tester.pump(_delay);
      await tester.pumpAndSettle();
    }
  }

  bool seen(String community) =>
      prefs.getBool(
        firstAgentCalloutKey(
          MeSync(FakeRelay(), RelayConfig(baseUrl: community, nsec: nsec)),
        ),
      ) ==
      true;

  testWidgets('shows above the empty Bestie slot without blocking Home', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await pumpHome(tester);
    expect(find.text(_card), findsOneWidget);
    expect(
      tester.getBottomLeft(find.text(_card)).dy,
      lessThan(tester.getTopLeft(find.byTooltip('Bestie')).dy),
    );
    expect(
      tester.getSemantics(find.text(_card)),
      matchesSemantics(
        label: _card,
        isButton: true,
        hasDismissAction: true,
        hasTapAction: true,
      ),
    );
    // The screen underneath still takes taps, and they leave the card up.
    await tester.tap(find.text('Channel'));
    await tester.pumpAndSettle();
    expect(otherTaps, 1);
    expect(find.text(_card), findsOneWidget);
    expect(seen('https://a.test'), isFalse);
    semantics.dispose();
  });

  testWidgets('activating the slot dismisses it and starts creation', (
    tester,
  ) async {
    await pumpHome(tester);
    // Screen readers and keyboards activate the slot without a touch.
    tester
        .widget<IconButton>(
          find.ancestor(
            of: find.byTooltip('Bestie'),
            matching: find.byType(IconButton),
          ),
        )
        .onPressed!();
    await tester.pumpAndSettle();
    expect(choices, 1);
    expect(find.text(_card), findsNothing);
    expect(seen('https://a.test'), isTrue);
  });

  testWidgets('tapping the card starts creation and retires it', (
    tester,
  ) async {
    await pumpHome(tester);
    await tester.tap(find.text(_card));
    await tester.pumpAndSettle();
    expect(choices, 1);
    expect(find.text(_card), findsNothing);
    expect(seen('https://a.test'), isTrue);
  });

  testWidgets('the close button dismisses it without creating', (tester) async {
    await pumpHome(tester);
    await tester.tap(find.byTooltip('Dismiss'));
    await tester.pumpAndSettle();
    expect(choices, 0);
    expect(find.text(_card), findsNothing);
    expect(seen('https://a.test'), isTrue);
  });

  testWidgets('a screen reader can dismiss it', (tester) async {
    final semantics = tester.ensureSemantics();
    await pumpHome(tester);
    tester.semantics.performAction(
      find.semantics.byLabel(_card),
      SemanticsAction.dismiss,
    );
    await tester.pumpAndSettle();
    expect(find.text(_card), findsNothing);
    expect(seen('https://a.test'), isTrue);
    semantics.dispose();
  });

  testWidgets('a Bestie retires it, even if the Bestie is later cleared', (
    tester,
  ) async {
    await pumpHome(tester);
    expect(find.text(_card), findsOneWidget);
    bestie.add('a' * 64);
    await tester.pumpAndSettle();
    expect(find.text(_card), findsNothing);
    expect(seen('https://a.test'), isTrue);
    bestie.add(null);
    await tester.pumpAndSettle();
    expect(find.text(_card), findsNothing);
  });

  testWidgets('once dismissed it never returns in that community', (
    tester,
  ) async {
    await pumpHome(tester);
    await tester.tap(find.byTooltip('Dismiss'));
    await tester.pumpAndSettle();
    await pumpHome(tester);
    expect(find.text(_card), findsNothing);
    // Another community is its own first run.
    await pumpHome(tester, community: 'https://b.test');
    expect(find.text(_card), findsOneWidget);
  });

  testWidgets('stays hidden for anyone who already has agents', (tester) async {
    await pumpHome(tester, owned: [OwnedAgentDirectoryEntry(pubkey: 'b' * 64)]);
    expect(find.text(_card), findsNothing);
    await pumpHome(tester, remoteAgents: false);
    expect(find.text(_card), findsNothing);
    // None of these count as dismissing it.
    await pumpHome(tester);
    expect(find.text(_card), findsOneWidget);
  });

  testWidgets('hides while anything covers Home, then returns', (tester) async {
    await pumpHome(tester);
    final navigator = tester.state<NavigatorState>(find.byType(Navigator));
    // A sheet leaves Home painted underneath, so only the route check hides
    // the tip there.
    unawaited(
      showModalBottomSheet<void>(
        context: navigator.context,
        builder: (_) => const Text('Sheet'),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Channel'), findsOneWidget);
    expect(find.text(_card), findsNothing);
    navigator.pop();
    await tester.pumpAndSettle();
    // Coming back restarts the wait.
    expect(find.text(_card), findsNothing);
    await tester.pump(_delay);
    await tester.pumpAndSettle();
    expect(find.text(_card), findsOneWidget);
    expect(seen('https://a.test'), isFalse);
  });

  testWidgets('waits three seconds on Home, then pops up', (tester) async {
    await pumpHome(tester, waitForCard: false);
    await tester.pump(_delay - const Duration(milliseconds: 100));
    expect(find.text(_card), findsNothing);
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pump();
    expect(find.text(_card), findsOneWidget);
    // It starts faint and fades in.
    expect(_opacity(tester), lessThan(1));
    await tester.pumpAndSettle();
    expect(_opacity(tester), 1);
  });

  testWidgets('with reduced motion it still waits, then appears at once', (
    tester,
  ) async {
    await pumpHome(tester, reducedMotion: true, waitForCard: false);
    await tester.pump(_delay - const Duration(milliseconds: 100));
    expect(find.text(_card), findsNothing);
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pump();
    expect(find.text(_card), findsOneWidget);
    expect(_opacity(tester), 1);
  });

  testWidgets('leaving Home during the wait keeps it from popping up', (
    tester,
  ) async {
    await pumpHome(tester, waitForCard: false);
    await tester.pump(const Duration(seconds: 2));
    final navigator = tester.state<NavigatorState>(find.byType(Navigator));
    unawaited(
      navigator.push(
        MaterialPageRoute<void>(builder: (_) => const Text('Elsewhere')),
      ),
    );
    await tester.pumpAndSettle();
    await tester.pump(_delay);
    await tester.pumpAndSettle();
    expect(find.text(_card), findsNothing);
    navigator.pop();
    await tester.pumpAndSettle();
    // Back on Home, the full wait starts over.
    await tester.pump(const Duration(seconds: 2));
    expect(find.text(_card), findsNothing);
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(find.text(_card), findsOneWidget);
  });
}

double _opacity(WidgetTester tester) => tester
    .widget<Opacity>(
      find.ancestor(of: find.text(_card), matching: find.byType(Opacity)).first,
    )
    .opacity;
