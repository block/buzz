import 'dart:async';

import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/channels/default_agent_voice_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/default_agent_writer.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/me/me_sync_test.dart' show FakeRelay;

class _EmptyVoiceDraft extends DefaultAgentVoiceDraft {
  _EmptyVoiceDraft(super.identity);
  @override
  Future<VoiceNoteRecording?> read() async => null;
}

void main() {
  testWidgets(
    'community switch cannot launch a retained default while loading',
    (tester) async {
      const secret =
          '0000000000000000000000000000000000000000000000000000000000000001';
      final nsec = nostr.Nip19.encode(
        prefix: nostr.Nip19Prefix.nsec,
        data: secret,
      );
      final relayA = FakeRelay();
      final relayB = FakeRelay();
      final a = MeSync(
        relayA,
        RelayConfig(baseUrl: 'https://a.test', nsec: nsec),
      );
      final b = MeSync(
        relayB,
        RelayConfig(baseUrl: 'https://b.test', nsec: nsec),
      );
      await saveDefaultAgent(a, 'a' * 64);
      final pending = Completer<List<NostrEvent>>();
      relayB.pendingRead = pending.future;
      var choices = 0;
      Widget app(MeSync scope) => ProviderScope(
        overrides: [
          defaultAgentVoiceDraftProvider.overrideWith(
            (ref, identity) => _EmptyVoiceDraft(identity),
          ),
          meSyncProvider.overrideWithValue(scope),
          relaySessionProvider.overrideWith(() => relayA),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: DefaultAgentLauncher(onChooseAgent: () => choices++),
          ),
        ),
      );
      await tester.pumpWidget(app(a));
      await tester.pumpAndSettle();
      final button = find.byType(IconButton);
      final staleAction = tester.widget<IconButton>(button).onPressed!;
      await tester.pumpWidget(app(b));
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(tester.element(button));
      expect(container.read(defaultAgentProvider).isLoading, isTrue);
      // Riverpod can keep A's previous data while B's first read is pending.
      expect(tester.widget<IconButton>(button).onPressed, isNull);
      staleAction();
      await tester.pumpAndSettle();
      expect(choices, 0);
      expect(relayB.writes, 0);
      pending.complete([]);
      await tester.pumpAndSettle();
      await tester.tap(button);
      expect(choices, 1);
      expect(tester.takeException(), isNull);
    },
  );
}
