import 'package:buzz/shared/me/hold_to_talk.dart';
import 'package:buzz/features/settings/settings_page.dart';
import 'dart:async';

import 'package:buzz/features/channels/default_agent_launcher.dart';
import 'package:buzz/features/channels/first_agent_callout.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/features/channels/default_agent_voice_control.dart';
import 'package:image_picker/image_picker.dart';
import 'package:buzz/features/channels/default_agent_voice_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/default_agent_writer.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/me/me_sync_test.dart' show FakeRelay;

class _EmptyVoiceDraft extends DefaultAgentVoiceDraft {
  _EmptyVoiceDraft(super.identity);
  @override
  Future<VoiceNoteRecording?> read() async => null;
}

class _SavedDraft extends DefaultAgentVoiceDraft {
  _SavedDraft(super.identity, this.notes, {super.onChanged});
  final Map<String, VoiceNoteRecording> notes;
  @override
  Future<VoiceNoteRecording?> read() async => notes[identity];
  @override
  Future<void> clear({String? generation}) async {
    notes.remove(identity);
    onChanged?.call();
  }
}

void main() {
  testWidgets(
    'saved recording survives default changes; unset Bestie ignores holds',
    (tester) async {
      final haptics = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          haptics.add(call);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      final scope = MeSync(
        relay,
        RelayConfig(
          baseUrl: 'https://draft.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '0' * 63 + '1',
          ),
        ),
      );
      final agent = 'a' * 64;
      final other = 'b' * 64;
      final identity = '${scope.community}:account:$agent';
      final notes = <String, VoiceNoteRecording>{
        identity: VoiceNoteRecording(
          file: XFile('/tmp/retained-note.m4a'),
          duration: const Duration(seconds: 2),
          waveform: const [.2],
        ),
      };
      var selected = agent as String?;
      final preferences = StreamController<String?>.broadcast(sync: true);
      addTearDown(preferences.close);
      Widget app() => ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          remoteAgentsEnabledProvider.overrideWithValue(false),
          voiceNoteRecorderFactoryProvider.overrideWithValue(
            () => throw StateError(
              'Unset Bestie must not request the microphone',
            ),
          ),
          defaultAgentProvider.overrideWith((ref) async* {
            yield selected;
            yield* preferences.stream;
          }),
          meSyncProvider.overrideWithValue(scope),
          myPubkeyProvider.overrideWithValue('account'),
          relaySessionProvider.overrideWith(FakeRelay.new),
          defaultAgentVoiceDraftProvider.overrideWith(
            (ref, identity) => _SavedDraft(
              identity,
              notes,
              onChanged: () =>
                  ref.invalidate(pendingAgentVoiceRecipientsProvider),
            ),
          ),
          pendingAgentVoiceRecipientsProvider.overrideWith((
            ref,
            scopeIdentity,
          ) async {
            ref.watch(defaultAgentProvider);
            return notes.keys
                .where((key) => key.startsWith('$scopeIdentity:'))
                .map((key) => key.substring(scopeIdentity.length + 1))
                .toList();
          }),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Column(
              children: [
                DefaultAgentLauncher(onChooseAgent: () {}),
                Builder(
                  builder: (context) => TextButton(
                    onPressed: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(
                        builder: (_) => SettingsPage(
                          profileHeader: const SizedBox.shrink(),
                          identityRecoveryPageBuilder: (_) =>
                              const SizedBox.shrink(),
                        ),
                      ),
                    ),
                    child: const Text('Settings'),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
      await tester.pumpWidget(app());
      await tester.pumpAndSettle();
      expect(find.text('Retry voice note'), findsOneWidget);
      await tester.tap(find.text('Settings'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Hold to talk to Bestie'));
      await tester.tap(find.text('Hold to talk to Bestie'));
      await tester.pumpAndSettle();
      expect(prefs.getBool(HoldToTalkPreference.storageKey), isFalse);
      await tester.tap(find.byTooltip('Close settings'));
      await tester.pumpAndSettle();
      expect(find.text('Retry voice note'), findsOneWidget);
      selected = other;
      preferences.add(selected);
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<DefaultAgentVoiceControl>(
              find.byType(DefaultAgentVoiceControl),
            )
            .draftIdentity,
        identity,
      );
      expect(find.byTooltip('Discard voice note'), findsOneWidget);
      selected = null;
      preferences.add(selected);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpWidget(app());
      await tester.pumpAndSettle();
      expect(find.text('Retry voice note'), findsOneWidget);
      expect(
        tester
            .widget<DefaultAgentVoiceControl>(
              find.byType(DefaultAgentVoiceControl),
            )
            .draftIdentity,
        identity,
      );
      await tester.tap(find.byTooltip('Discard voice note'));
      await tester.pumpAndSettle();
      expect(notes, isEmpty);
      selected = agent;
      preferences.add(selected);
      await tester.pumpAndSettle();
      haptics.clear();
      await tester.longPress(find.byType(DefaultAgentLauncher));
      await tester.pumpAndSettle();
      expect(haptics, isEmpty);
      expect(tester.takeException(), isNull);
      final container = ProviderScope.containerOf(
        tester.element(find.byType(DefaultAgentLauncher)),
      );
      expect(container.read(holdToTalkProvider), isFalse);
      await container.read(holdToTalkProvider.notifier).setEnabled(true);
      selected = null;
      preferences.add(selected);
      await tester.pumpAndSettle();
      haptics.clear();
      await tester.longPress(find.byTooltip('Bestie'));
      await tester.pumpAndSettle();
      expect(find.text('Slide up to cancel'), findsNothing);
      expect(haptics, isEmpty);
      expect(
        (ProviderScope.containerOf(
                  tester.element(find.byType(DefaultAgentLauncher)),
                ).read(relaySessionProvider.notifier)
                as FakeRelay)
            .writes,
        0,
      );
      final semantics = tester.ensureSemantics();
      expect(find.bySemanticsLabel('Record voice note'), findsNothing);
      semantics.dispose();
      expect(find.text('Retry voice note'), findsNothing);
      expect(tester.takeException(), isNull);
    },
  );
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
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      Widget app(MeSync scope) => ProviderScope(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          defaultAgentVoiceDraftProvider.overrideWith(
            (ref, identity) => _EmptyVoiceDraft(identity),
          ),
          pendingAgentVoiceRecipientsProvider.overrideWith(
            (ref, scope) async => [],
          ),
          meSyncProvider.overrideWithValue(scope),
          relaySessionProvider.overrideWith(() => relayA),
          remoteAgentsEnabledProvider.overrideWithValue(true),
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
      // A failed read can keep A's Bestie as B's value. That must not retire
      // the first-agent card in B, which has no Bestie.
      pending.completeError(StateError('offline'));
      await tester.pumpAndSettle();
      expect(container.read(defaultAgentProvider).hasError, isTrue);
      expect(prefs.getBool(firstAgentCalloutKey(b)), isNot(true));
      relayB.pendingRead = null;
      await tester.tap(button);
      await tester.pumpAndSettle();
      expect(choices, 0);
      expect(prefs.getBool(firstAgentCalloutKey(b)), isNot(true));
      await tester.tap(button);
      expect(choices, 1);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
