part of '../compose_bar_test.dart';

class _TrayPickerService extends _FakeVoiceNoteUploadService {
  int photoOpens = 0;
  @override
  Future<List<XFile>> pickGalleryImages() async {
    photoOpens++;
    return [];
  }
}

class _ComposerSpeechFiles extends ComposerTranscriptionFiles {
  int nextId = 0;
  bool rejectRetention = false;
  bool rejectDeletion = false;
  bool rejectSourceDeletion = false;
  int retainCalls = 0;
  Completer<void>? finishRetention;
  final deleted = <String>[];
  @override
  Future<String> retain(
    String source,
    Future<void> Function(String) reserve,
  ) async {
    retainCalls++;
    await finishRetention?.future;
    if (rejectRetention) throw StateError('Storage unavailable');
    final copy = '$source.${nextId++}.saved';
    await reserve(copy);
    return copy;
  }

  @override
  Future<void> delete(String path) async {
    if ((rejectDeletion && path.endsWith('.saved')) ||
        (rejectSourceDeletion && !path.endsWith('.saved'))) {
      throw StateError('Delete failed');
    }
    deleted.add(path);
  }
}

class _DeferredStopDictationRecorder extends _FakeVoiceNoteRecorder {
  final finishStop = Completer<void>();
  @override
  Future<VoiceNoteRecording> stop() async {
    await finishStop.future;
    return super.stop();
  }
}

class _ComposerSpeechPrefs extends Fake implements SharedPreferences {
  _ComposerSpeechPrefs(this.delegate);

  final SharedPreferences delegate;
  Completer<void>? holdNextDraftWrite;
  bool rejectDraftWrites = false;
  bool rejectRecoveryWrites = false;
  bool rejectRetainedWrites = false;
  bool rejectRemoval = false;
  bool rejectCleanupRemoval = false;
  final hiddenKeys = <String>{};
  final cachedWrites = <String, String>{};
  int removalCalls = 0;

  @override
  Future<void> reload() async {
    hiddenKeys.clear();
    cachedWrites.clear();
    await delegate.reload();
  }

  @override
  String? getString(String key) => hiddenKeys.contains(key)
      ? null
      : cachedWrites[key] ?? delegate.getString(key);
  @override
  bool? getBool(String key) => delegate.getBool(key);
  @override
  List<String>? getStringList(String key) => delegate.getStringList(key);
  @override
  Set<String> getKeys() => delegate.getKeys();
  @override
  Future<bool> remove(String key) async {
    if (rejectCleanupRemoval &&
        key.startsWith('composer_transcription_cleanup:')) {
      hiddenKeys.add(key);
      return false;
    }
    if (key.startsWith('composer_transcription:')) {
      removalCalls++;
      if (rejectRemoval) {
        hiddenKeys.add(
          key,
        ); // Model SharedPreferences cache-before-platform behavior.
        return false;
      }
    }
    return delegate.remove(key);
  }

  @override
  Future<bool> setString(String key, String value) async {
    if (key.startsWith('compose_drafts_v1:') && holdNextDraftWrite != null) {
      final hold = holdNextDraftWrite!;
      holdNextDraftWrite = null;
      await hold.future;
    }
    if ((rejectDraftWrites && key.startsWith('compose_drafts_v1:')) ||
        (rejectRecoveryWrites && key.startsWith('composer_transcription:')) ||
        (rejectRetainedWrites &&
            key.startsWith('composer_transcription:') &&
            !value.startsWith('pending-source:'))) {
      cachedWrites[key] = value;
      return false;
    }
    cachedWrites.remove(key);
    return delegate.setString(key, value);
  }
}

class _ComposerSpeech implements DeviceTranscription {
  Completer<String> result = Completer<String>();
  Completer<void> started = Completer<void>();
  Completer<void> cancelled = Completer<void>();
  Completer<void>? finishCancelling;
  bool available = true;
  @override
  Future<bool> isAvailable() async => available;
  @override
  Future<void> prepare() async {}
  @override
  Future<String> transcribe(String path) {
    started.complete();
    return result.future;
  }

  @override
  Future<void> cancel() async {
    if (!cancelled.isCompleted) cancelled.complete();
    await finishCancelling?.future;
  }

  void reset() {
    result = Completer<String>();
    started = Completer<void>();
    cancelled = Completer<void>();
    finishCancelling = null;
  }
}

void iosControlsTests() {
  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/transcription'),
          (call) async => call.method == 'isAvailable' ? true : null,
        );
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/native_attachment_popover'),
          (call) async => false,
        );
  });
  tearDown(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/transcription'),
          null,
        );
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('buzz/native_attachment_popover'),
          null,
        );
  });
  testWidgets(
    'microphone keeps recording controls and inserts a retryable transcript',
    (tester) async {
      final speech = _ComposerSpeech()..available = false;
      final files = _ComposerSpeechFiles();
      final prefs = _ComposerSpeechPrefs(_testPrefs);
      _testPrefs = prefs;
      final upload = _FakeVoiceNoteUploadService();
      var captures = 0;
      var sends = 0;
      Widget composer({String? thread}) => _buildComposeBar(
        uploadService: upload,
        threadHeadId: thread,
        voiceNoteRecorderFactory: () {
          captures++;
          return _FakeVoiceNoteRecorder();
        },
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
        onSend: (_, _, {mediaTags = const []}) async {
          sends++;
        },
      );
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      expect(captures, 0);
      expect(
        find.text('Dictation isn’t available on this device or language.'),
        findsOneWidget,
      );
      speech.available = true;
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('voice-note-recorder')), findsOneWidget);
      expect(find.byTooltip('Stop recording'), findsOneWidget);
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      expect(speech.started.isCompleted, isTrue);
      await tester.pump();
      expect(find.text('Transcribing…'), findsOneWidget);
      speech.result.completeError(StateError('Speech service unavailable.'));
      await tester.pump();
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsOneWidget);
      expect(
        _testPrefs.getKeys().where(
          (key) => key.startsWith('composer_transcription:'),
        ),
        hasLength(1),
      );
      speech.reset();
      await tester.tap(find.text('Retry'));
      await tester.pump();
      expect(speech.started.isCompleted, isTrue);
      prefs.rejectDraftWrites = true;
      speech.result.complete('Meet me at ten.');
      await tester.pumpAndSettle();
      expect(
        find.text('Could not save the draft. Your recording is saved.'),
        findsOneWidget,
      );
      expect(files.deleted.where((path) => path.endsWith('.saved')), isEmpty);
      // Reenter with the same in-memory draft after its write failed.
      await tester.pumpWidget(composer(thread: 'other-thread'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      prefs.rejectDraftWrites = false;
      speech.reset();
      await tester.tap(find.text('Retry'));
      await tester.pump();
      await tester.pumpAndSettle();
      expect(speech.started.isCompleted, isFalse);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        'Meet me at ten.',
      );
      expect(find.byTooltip('Remove attachment'), findsNothing);
      expect(sends, 0);
      expect(
        _testPrefs.getKeys().where(
          (key) => key.startsWith('composer_transcription:'),
        ),
        isEmpty,
      );

      // A late recognition belongs to its original conversation, never the new one.
      await tester.enterText(find.byType(TextField), '');
      await tester.pumpAndSettle();
      speech.reset();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      expect(speech.started.isCompleted, isTrue);
      await tester.pumpWidget(composer(thread: 'another-thread'));
      await tester.pump();
      speech.result.complete('Belongs to the previous draft.');
      await tester.pump();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Message'));
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        isEmpty,
      );
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsOneWidget);
      final originalKey = prefs.getKeys().singleWhere(
        (key) => key.startsWith('composer_transcription:'),
      );
      final originalPath = prefs.getString(originalKey)!;
      final threadKey = '$originalKey:another-thread';
      const threadPath = '/tmp/thread-recording.saved';
      await prefs.setString(threadKey, threadPath);

      speech.reset();
      await tester.tap(find.text('Retry'));
      await tester.pump();
      expect(speech.started.isCompleted, isTrue);
      final finishCancelling = speech.finishCancelling = Completer<void>();
      await tester.tap(find.byTooltip('Discard recording'));
      await tester.pump();
      expect(speech.cancelled.isCompleted, isTrue);
      await tester.pumpWidget(composer(thread: 'another-thread'));
      await tester.pump();
      finishCancelling.complete();
      speech.result.complete('Discarded recording.');
      await tester.pumpAndSettle();
      expect(prefs.getString(originalKey), isNull);
      expect(files.deleted, contains(originalPath));
      expect(prefs.getString(threadKey), threadPath);
      expect(files.deleted, isNot(contains(threadPath)));
      expect(find.text('Retry'), findsOneWidget);
      // A Discard that fails at pointer removal leaves the recording to
      // transcribe: Retry recognizes it before any deletion.
      prefs.rejectRemoval = true;
      await tester.tap(find.byTooltip('Discard recording'));
      await tester.pumpAndSettle();
      expect(
        find.text('Could not discard the recording. Try again.'),
        findsOneWidget,
      );
      prefs.rejectRemoval = false;
      speech.reset();
      await tester.tap(find.text('Retry'));
      await tester.pump();
      expect(speech.started.isCompleted, isTrue);
      expect(files.deleted, isNot(contains(threadPath)));
      speech.result.complete('Kept after a failed discard.');
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsNothing);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        'Kept after a failed discard.',
      );
      expect(files.deleted, contains(threadPath));
      expect(sends, 0);
      await tester.pumpWidget(const SizedBox.shrink());

      // A reopened conversation shows its agent prefill again, behind the
      // saved recording. Retry appends to it and keeps its exact recipient.
      final viewer = 'f' * 64;
      final agent = 'a' * 64;
      final namesake = 'b' * 64;
      final agentIdentity = RelayConfig(
        baseUrl: 'http://localhost:3000',
        nsec: nostr.Keys.generate().nsec,
      );
      var renamed = false;
      List<String>? recipients;
      Widget agentComposer() => _buildComposeBar(
        currentPubkey: viewer,
        channels: [_makeCurrentChannel()],
        relayConfig: () => _SwitchableRelayConfigNotifier(agentIdentity),
        uploadService: upload,
        voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
        loadMembers: () async => [
          ChannelMember(
            pubkey: agent,
            displayName: renamed ? 'Ranger' : 'Scout',
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
          ChannelMember(
            pubkey: namesake,
            displayName: renamed ? 'Scout' : 'Pat',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        prefillScopeMessages: [
          TimelineMessage(
            id: 'own-message-to-agent',
            pubkey: viewer,
            createdAt: 10,
            content: '',
            tags: [
              const ['h', 'channel-1'],
              ['p', agent],
            ],
          ),
        ],
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
        onSend: (_, mentions, {mediaTags = const []}) async {
          recipients = mentions;
        },
      );
      ComposeDraft? agentDraft() => ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      ).read(composeDraftsProvider.notifier).draftFor('channel-1');
      speech.reset();
      await tester.pumpWidget(agentComposer());
      await tester.pumpAndSettle();
      // The mic is offered only for an empty field: remove the prefill.
      await tester.tap(find.text('@Scout '));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '');
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      speech.result.completeError(StateError('Speech service unavailable.'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpWidget(agentComposer());
      await tester.pumpAndSettle();
      expect(agentDraft(), isNull);
      speech.reset();
      await tester.tap(find.text('Retry'));
      await tester.pump();
      speech.result.complete('Keep the exact recipient.');
      await tester.pumpAndSettle();
      expect(agentDraft()?.text, '@Scout Keep the exact recipient.');
      expect(agentDraft()?.mentions.map((m) => (m.start, m.end, m.pubkey)), [
        (0, 6, agent),
      ]);
      expect(agentDraft()?.transcriptionId, endsWith('.saved'));
      // The agent is renamed and another member takes its old name.
      renamed = true;
      ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      ).invalidate(channelMembersProvider('channel-1'));
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Send');
      await tester.pumpAndSettle();
      expect(recipients, [agent]);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  for (final failure in ['copy', 'source-pointer', 'retained-pointer']) {
    testWidgets(
      'retention $failure failure retries before recognition after remount',
      (tester) async {
        final identity = RelayConfig(
          baseUrl: 'http://localhost:3000',
          nsec: nostr.Keys.generate().nsec,
        );
        final speech = _ComposerSpeech();
        final files = _ComposerSpeechFiles()
          ..rejectRetention = failure == 'copy'
          ..rejectDeletion = failure == 'retained-pointer';
        final prefs = _ComposerSpeechPrefs(_testPrefs)
          ..rejectRecoveryWrites = failure == 'source-pointer'
          ..rejectRetainedWrites = failure == 'retained-pointer';
        _testPrefs = prefs;
        Widget composer({String? thread}) => _buildComposeBar(
          threadHeadId: thread,
          relayConfig: () => _SwitchableRelayConfigNotifier(identity),
          uploadService: _FakeVoiceNoteUploadService(),
          onSend: (_, _, {mediaTags = const []}) async {},
          voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
          overrides: [
            composerTranscriptionFilesProvider.overrideWithValue(files),
            deviceTranscriptionProvider.overrideWithValue(() => speech),
          ],
        );
        await tester.pumpWidget(composer());
        await tester.pumpAndSettle();
        await _tapComposerControl(tester, 'Dictate');
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Stop recording'));
        await tester.pumpAndSettle();
        expect(speech.started.isCompleted, isFalse);
        expect(
          find.text(
            'Recording not safely saved. Retry before closing the app.',
          ),
          findsOneWidget,
        );
        if (failure == 'retained-pointer') {
          expect(files.deleted.where((p) => p.endsWith('.saved')), isEmpty);
          files.rejectDeletion = false;
        }
        expect(files.deleted, isNot(contains('/tmp/voice-note-test.m4a')));
        // A failed copy/replacement pointer has a persisted source journal;
        // recreate the provider container to model restart. A completely failed
        // preferences write can only survive navigation within this session.
        if (failure != 'source-pointer') {
          await tester.pumpWidget(const SizedBox.shrink());
          await tester.pumpAndSettle();
          await prefs.reload();
        } else {
          await tester.pumpWidget(composer(thread: 'other-thread'));
          await tester.pumpAndSettle();
        }
        await tester.pumpWidget(composer());
        await tester.pumpAndSettle();
        expect(find.text('Retry'), findsOneWidget);
        if (failure == 'retained-pointer') {
          expect(files.deleted, contains('/tmp/voice-note-test.m4a.0.saved'));
        }
        if (failure == 'copy') {
          // A Discard that fails at pointer removal journals the source while
          // the pending-source pointer still owns it: a restart keeps it.
          prefs.rejectRemoval = true;
          await tester.tap(find.byTooltip('Discard recording'));
          await tester.pumpAndSettle();
          prefs.rejectRemoval = false;
          await tester.pumpWidget(const SizedBox.shrink());
          await tester.pumpAndSettle();
          await prefs.reload();
          await tester.pumpWidget(composer());
          await tester.pumpAndSettle();
          expect(files.deleted, isNot(contains('/tmp/voice-note-test.m4a')));
          expect(find.text('Retry'), findsOneWidget);
        }
        files.rejectRetention = false;
        prefs.rejectRecoveryWrites = false;
        prefs.rejectRetainedWrites = false;
        files.rejectSourceDeletion = true;
        final priorCopies = files.retainCalls;
        await tester.tap(find.text('Retry'));
        await tester.pump();
        expect(files.retainCalls, priorCopies + 1);
        expect(speech.started.isCompleted, isTrue);
        final key = prefs.getKeys().singleWhere(
          (k) => k.startsWith('composer_transcription:'),
        );
        expect(prefs.getString(key), endsWith('.saved'));
        speech.result.complete('Recovered once.');
        await tester.pumpAndSettle();
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          'Recovered once.',
        );
        expect(prefs.getString(key), isNull);
        const original = '/tmp/voice-note-test.m4a';
        expect(files.deleted, isNot(contains(original)));
        await tester.pumpWidget(const SizedBox.shrink());
        await prefs.reload();
        files.rejectSourceDeletion = false;
        await tester.pumpWidget(composer());
        await tester.pumpAndSettle();
        expect(files.deleted, contains(original));
        expect(find.text('Retry'), findsNothing);
        await tester.pumpWidget(const SizedBox.shrink());
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }

  testWidgets(
    'rebuilding draft provider does not treat failed cached write as durable',
    (tester) async {
      final identity = RelayConfig(
        baseUrl: 'http://localhost:3000',
        nsec: nostr.Keys.generate().nsec,
      );
      final speech = _ComposerSpeech();
      final files = _ComposerSpeechFiles();
      final prefs = _ComposerSpeechPrefs(_testPrefs)..rejectDraftWrites = true;
      _testPrefs = prefs;
      Widget composer() => _buildComposeBar(
        relayConfig: () => _SwitchableRelayConfigNotifier(identity),
        uploadService: _FakeVoiceNoteUploadService(),
        onSend: (_, _, {mediaTags = const []}) async {},
        voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
      );
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      speech.result.complete('Keep this draft.');
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      // Keep the preferences cache, as when switching accounts and back.
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      speech.reset();
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      expect(speech.started.isCompleted, isFalse);
      expect(
        find.text('Could not save the draft. Your recording is saved.'),
        findsOneWidget,
      );
      expect(files.deleted.where((p) => p.endsWith('.saved')), isEmpty);
      prefs.rejectDraftWrites = false;
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      await prefs.reload();
      final key = prefs.getKeys().singleWhere(
        (k) => k.startsWith('compose_drafts_v1:'),
      );
      expect(prefs.getString(key), contains('Keep this draft.'));
      expect(files.deleted.where((p) => p.endsWith('.saved')), hasLength(1));
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets(
    'a retired draft save preserves the next recording after restart',
    (tester) async {
      final speech = _ComposerSpeech();
      final files = _ComposerSpeechFiles();
      final prefs = _ComposerSpeechPrefs(_testPrefs);
      _testPrefs = prefs;
      final identity = RelayConfig(
        baseUrl: 'http://localhost:3000',
        nsec: nostr.Keys.generate().nsec,
      );
      Widget composer(String mount) => _buildComposeBar(
        composeBarKey: mount,
        relayConfig: () => _SwitchableRelayConfigNotifier(identity),
        uploadService: _FakeVoiceNoteUploadService(),
        onSend: (_, _, {mediaTags = const []}) async {},
        voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
      );
      Future<void> dictate() async {
        await _tapComposerControl(tester, 'Dictate');
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Stop recording'));
        await tester.pump();
      }

      await tester.pumpWidget(composer('first'));
      await tester.pumpAndSettle();
      await dictate();
      final hold = Completer<void>();
      prefs.holdNextDraftWrite = hold;
      speech.result.complete('First');
      await tester.pump();
      expect(prefs.holdNextDraftWrite, isNull);
      await tester.pumpWidget(composer('second'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Discard recording'));
      await tester.pumpAndSettle();
      if (find.byType(TextField).evaluate().isEmpty) {
        await tester.tap(find.text('First'));
        await tester.pumpAndSettle();
      }
      await _tapComposerControl(tester, 'Send');
      await tester.pumpAndSettle();
      speech.reset();
      await dictate();
      final key = prefs.getKeys().singleWhere(
        (key) => key.startsWith('composer_transcription:'),
      );
      final second = prefs.getString(key)!;
      hold.complete();
      speech.result.completeError(StateError('Recognition failed'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(const SizedBox.shrink());
      await prefs.reload();
      expect(prefs.getString(key), second);
      expect(files.deleted, isNot(contains(second)));
      await tester.pumpWidget(composer('restart'));
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  for (final discard in [false, true]) {
    testWidgets('reopening during retention shares copy, discard: $discard', (
      tester,
    ) async {
      final speech = _ComposerSpeech();
      final finish = Completer<void>();
      final files = _ComposerSpeechFiles()..finishRetention = finish;
      Widget composer({String? thread}) => _buildComposeBar(
        threadHeadId: thread,
        uploadService: _FakeVoiceNoteUploadService(),
        onSend: (_, _, {mediaTags = const []}) async {},
        voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
      );
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      expect(files.retainCalls, 1);
      await tester.pumpWidget(composer(thread: 'other-thread'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await tester.tap(
        discard ? find.byTooltip('Discard recording') : find.text('Retry'),
      );
      await tester.pump();
      expect(files.retainCalls, 1);
      finish.complete();
      await tester.pump();
      if (!discard) {
        expect(speech.started.isCompleted, isTrue);
        speech.result.complete('Shared recording.');
      }
      await tester.pumpAndSettle();
      expect(files.retainCalls, 1);
      expect(files.deleted.where((p) => p.endsWith('.saved')), hasLength(1));
      expect(find.text('Retry'), findsNothing);
      expect(
        ProviderScope.containerOf(
          tester.element(find.byType(ComposeBar)),
        ).read(composeDraftsProvider.notifier).textFor('channel-1'),
        discard ? null : 'Shared recording.',
      );
      await tester.pumpWidget(const SizedBox.shrink());
    }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
  }

  testWidgets(
    'failed recovery removal survives restart without duplicate transcript',
    (tester) async {
      final identity = RelayConfig(
        baseUrl: 'http://localhost:3000',
        nsec: nostr.Keys.generate().nsec,
      );
      final speech = _ComposerSpeech();
      final files = _ComposerSpeechFiles();
      final prefs = _ComposerSpeechPrefs(_testPrefs)..rejectRemoval = true;
      _testPrefs = prefs;
      Widget composer() => _buildComposeBar(
        relayConfig: () => _SwitchableRelayConfigNotifier(identity),
        uploadService: _FakeVoiceNoteUploadService(),
        onSend: (_, _, {mediaTags = const []}) async {},
        voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
      );
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      speech.result.complete('Only once.');
      await tester.pumpAndSettle();
      final key = prefs.getKeys().singleWhere(
        (k) => k.startsWith('composer_transcription:'),
      );
      final audio = prefs.delegate.getString(key)!;
      expect(files.deleted, isNot(contains(audio)));
      expect(prefs.removalCalls, 1);
      // Retry must issue another durable remove even if a previous call cleared
      // its cache. Then cold-start from persisted draft and recovery state.
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      expect(prefs.removalCalls, 2);
      expect(files.deleted, isNot(contains(audio)));
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      await prefs.reload();
      speech.reset();
      prefs.rejectRemoval = false;
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      expect(speech.started.isCompleted, isFalse);
      expect(
        ProviderScope.containerOf(
          tester.element(find.byType(ComposeBar)),
        ).read(composeDraftsProvider.notifier).textFor('channel-1'),
        'Only once.',
      );
      expect(files.deleted, contains(audio));
      await prefs.reload();
      expect(prefs.getString(key), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('stopped dictation survives route cover and editor disposal', (
    tester,
  ) async {
    final identity = RelayConfig(
      baseUrl: 'http://localhost:3000',
      nsec: nostr.Keys.generate().nsec,
    );
    final speech = _ComposerSpeech();
    final files = _ComposerSpeechFiles();
    final recorder = _DeferredStopDictationRecorder();
    final recorders = <VoiceNoteRecorder>[_FakeVoiceNoteRecorder(), recorder];
    final sent = <String>[];
    final visible = ValueNotifier(true);
    addTearDown(visible.dispose);
    Widget composer() => _buildComposeBar(
      wrapPage: (page) => ValueListenableBuilder<bool>(
        valueListenable: visible,
        builder: (_, show, _) => show ? page : const SizedBox.shrink(),
      ),
      relayConfig: () => _SwitchableRelayConfigNotifier(identity),
      uploadService: _FakeVoiceNoteUploadService(),
      onSend: (text, _, {mediaTags = const []}) async => sent.add(text),
      voiceNoteRecorderFactory: () => recorders.removeAt(0),
      overrides: [
        composerTranscriptionFilesProvider.overrideWithValue(files),
        deviceTranscriptionProvider.overrideWithValue(() => speech),
      ],
    );
    await tester.pumpWidget(composer());
    await tester.pumpAndSettle();
    // Discard during recognition gives the editor back before the cancelled
    // recognition replies.
    await _tapComposerControl(tester, 'Dictate');
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Stop recording'));
    await tester.pump();
    await tester.pump();
    expect(speech.started.isCompleted, isTrue);
    final stale = speech.result;
    await tester.tap(find.byTooltip('Discard recording'));
    await tester.pumpAndSettle();
    expect(find.text('Saving recording…'), findsNothing);
    await tester.tap(find.text('Message'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Typed after discarding.');
    await tester.pumpAndSettle();
    await _tapComposerControl(tester, 'Send');
    await tester.pumpAndSettle();
    expect(sent, ['Typed after discarding.']);
    speech.reset();
    files.finishRetention = Completer<void>();
    await _tapComposerControl(tester, 'Dictate');
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Stop recording'));
    await tester.pump();
    final navigator = Navigator.of(tester.element(find.byType(ComposeBar)));
    unawaited(
      navigator.push(
        MaterialPageRoute<void>(
          builder: (_) => const Scaffold(body: Text('Covered')),
        ),
      ),
    );
    await tester.pumpAndSettle();
    visible.value = false;
    navigator.pop();
    await tester.pumpAndSettle();
    visible.value = true;
    await tester.pumpAndSettle();
    expect(find.text('Saving recording…'), findsOneWidget);
    expect(find.byTooltip('Dictate'), findsNothing);
    expect(find.byTooltip('Stop recording'), findsNothing);
    // The discarded recognition's late reply must not release this Stop.
    stale.complete('Stale reply');
    await tester.pumpAndSettle();
    expect(find.text('Saving recording…'), findsOneWidget);
    expect(find.byTooltip('Dictate'), findsNothing);
    expect(find.byTooltip('Stop recording'), findsNothing);
    recorder.finishStop.complete();
    await tester.pumpAndSettle();
    expect(find.text('Saving recording…'), findsOneWidget);
    files.finishRetention!.complete();
    await tester.pumpAndSettle();
    expect(files.retainCalls, 2);
    expect(speech.started.isCompleted, isFalse);
    expect(find.text('Retry'), findsOneWidget);
    await tester.tap(find.text('Retry'));
    await tester.pump();
    speech.result.complete('Saved after leaving.');
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      'Saved after leaving.',
    );
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets(
    'failed audio deletion keeps the composer and cleans up on a later mount',
    (tester) async {
      final identity = RelayConfig(
        baseUrl: 'http://localhost:3000',
        nsec: nostr.Keys.generate().nsec,
      );
      final speech = _ComposerSpeech();
      final files = _ComposerSpeechFiles()..rejectDeletion = true;
      final prefs = _ComposerSpeechPrefs(_testPrefs);
      _testPrefs = prefs;
      final sent = <String>[];
      Widget composer() => _buildComposeBar(
        relayConfig: () => _SwitchableRelayConfigNotifier(identity),
        uploadService: _FakeVoiceNoteUploadService(),
        onSend: (text, _, {mediaTags = const []}) async => sent.add(text),
        voiceNoteRecorderFactory: _FakeVoiceNoteRecorder.new,
        overrides: [
          composerTranscriptionFilesProvider.overrideWithValue(files),
          deviceTranscriptionProvider.overrideWithValue(() => speech),
        ],
      );
      String editorText() =>
          tester.widget<TextField>(find.byType(TextField)).controller!.text;
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      speech.result.complete('Text is already saved.');
      await tester.pumpAndSettle();
      expect(editorText(), 'Text is already saved.');
      expect(find.text('Retry'), findsNothing);
      expect(find.byTooltip('Discard recording'), findsNothing);
      final cleanupKey = _testPrefs.getKeys().singleWhere(
        (key) => key.startsWith('composer_transcription_cleanup:'),
      );
      final audio =
          (jsonDecode(_testPrefs.getString(cleanupKey)!) as List).single
              as String;
      expect(files.deleted, isNot(contains(audio)));
      // Also cover migration of an older installation's single-path journal.
      await _testPrefs.setString(cleanupKey, audio);
      // Restart with deletion still failing: the journal stays, quietly.
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      await _testPrefs.reload();
      speech.reset();
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsNothing);
      expect(find.text('Text is already saved.'), findsOneWidget);
      expect(_testPrefs.getString(cleanupKey), audio);
      expect(speech.started.isCompleted, isFalse);
      // Send remains available, and a second failed deletion must not replace
      // the first recording's durable cleanup entry.
      await _tapComposerControl(tester, 'Send');
      await tester.pumpAndSettle();
      speech.reset();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      speech.result.complete('Second recording.');
      await tester.pumpAndSettle();
      await _testPrefs.reload();
      final queued = (jsonDecode(_testPrefs.getString(cleanupKey)!) as List)
          .cast<String>();
      expect(queued, hasLength(2));
      expect(queued, contains(audio));
      expect(editorText(), 'Second recording.');
      expect(find.text('Retry'), findsNothing);
      speech.reset();
      // Once deletion works, the next mount removes both files and the journal.
      files.rejectDeletion = false;
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      await _testPrefs.reload();
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      expect(files.deleted, containsAll(queued));
      await _testPrefs.reload();
      expect(_testPrefs.getString(cleanupKey), isNull);
      expect(speech.started.isCompleted, isFalse);
      expect(find.text('Second recording.'), findsOneWidget);
      await _tapComposerControl(tester, 'Send');
      await tester.pumpAndSettle();
      expect(sent, ['Text is already saved.', 'Second recording.']);
      // The file deletes but its journal removal fails: the transcript still
      // lands in the editor, and a later mount removes the journal.
      prefs.rejectCleanupRemoval = true;
      speech.reset();
      await _tapComposerControl(tester, 'Dictate');
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Stop recording'));
      await tester.pump();
      speech.result.complete('Journal removal failed.');
      await tester.pumpAndSettle();
      expect(editorText(), 'Journal removal failed.');
      expect(find.text('Retry'), findsNothing);
      expect(_testPrefs.getString(cleanupKey), isNotNull);
      prefs.rejectCleanupRemoval = false;
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      await _testPrefs.reload();
      await tester.pumpWidget(composer());
      await tester.pumpAndSettle();
      await _testPrefs.reload();
      expect(_testPrefs.getString(cleanupKey), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('inactive and active empty iOS composer use a capsule', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(320, 700);
    addTearDown(tester.view.reset);
    const hint = 'Message #a-channel-with-a-very-long-name-that-must-not-wrap';
    final recorder = _FakeVoiceNoteRecorder();
    await tester.pumpWidget(
      _buildComposeBar(
        hintText: hint,
        voiceNoteRecorderFactory: () => recorder,
        uploadService: _TrayPickerService(),
        onSend: (_, _, {mediaTags = const []}) async {},
      ),
    );
    final surface = find.byKey(
      const ValueKey('composer-ios-concentric-surface'),
    );
    expect(tester.widget<ConcentricSheetSurface>(surface).capsule, isTrue);
    await _tapComposerControl(tester, 'Attachments');
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('attachment-menu-item-photos')),
      findsOneWidget,
    );
    expect(find.byType(TextField), findsNothing);
    await tester.tapAt(const Offset(10, 10));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Photos'), findsNothing);
    await tester.tap(find.text(hint));
    await tester.pumpAndSettle();
    expect(tester.widget<ConcentricSheetSurface>(surface).capsule, isTrue);
    expect(tester.getSize(find.text(hint)).height, lessThan(30));
    final plus = find.byKey(const ValueKey('ios-composer-plus'));
    final composer = find.byKey(const ValueKey('composer-surface'));
    expect(tester.getSize(plus).height, tester.getSize(composer).height);
    expect(tester.getTopLeft(composer).dx - tester.getTopRight(plus).dx, 8);
    expect(find.byTooltip('Send'), findsNothing);
    expect(find.byTooltip('Emoji'), findsOneWidget);
    expect(find.byTooltip('Dictate'), findsOneWidget);
    final emptyComposerSize = tester.getSize(composer);
    await tester.enterText(find.byType(TextField), 'A draft');
    await tester.pumpAndSettle();
    expect(tester.widget<ConcentricSheetSurface>(surface).capsule, isFalse);
    expect(tester.getSize(composer).width, emptyComposerSize.width);
    final fieldBounds = tester.getRect(find.byType(EditableText));
    final bubbleBounds = tester.getRect(composer);
    expect(fieldBounds.left - bubbleBounds.left, closeTo(14, 0.5));
    expect(fieldBounds.top - bubbleBounds.top, closeTo(10, 0.5));
    expect(bubbleBounds.bottom - fieldBounds.bottom, closeTo(10, 0.5));
    expect(find.byTooltip('Send'), findsOneWidget);
    expect(find.byTooltip('Emoji'), findsOneWidget);
    expect(find.byTooltip('Dictate'), findsNothing);
    await tester.enterText(find.byType(TextField), '');
    await tester.pumpAndSettle();
    expect(find.byTooltip('Send'), findsNothing);
    expect(find.byTooltip('Emoji'), findsOneWidget);
    expect(find.byTooltip('Dictate'), findsOneWidget);
    await _tapComposerControl(tester, 'Dictate');
    await tester.pumpAndSettle();
    expect(recorder.started, isTrue);
    expect(find.byKey(const ValueKey('voice-note-recorder')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('voice-note-recorder-close')));
    await tester.pumpAndSettle();
    expect(recorder.cancelled, isTrue);
    expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('selected text opens native Format after edit menu dismissal', (
    tester,
  ) async {
    final menus = _captureNativeComposerMenus();
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _TrayPickerService(),
        supportsShowingSystemContextMenu: true,
        onSend: (_, _, {mediaTags = const []}) async {},
      ),
    );
    await _expandComposer(tester);
    final editable = tester.state<EditableTextState>(find.byType(EditableText));
    final controller = tester
        .widget<TextField>(find.byType(TextField))
        .controller!;
    for (final style in [
      ('Bold', '**word**'),
      ('Italic', '_word_'),
      ('Strikethrough', '~~word~~'),
      ('Code', '`word`'),
      ('Code block', '```\nword\n```'),
    ]) {
      controller.value = const TextEditingValue(
        text: 'word',
        selection: TextSelection(baseOffset: 0, extentOffset: 4),
      );
      await tester.pump();
      editable.showToolbar();
      await tester.pumpAndSettle();
      expect(menus.last.first['title'], 'Format');
      final format = menus.last.singleWhere(
        (item) => item['title'] == 'Format',
      );
      await _nativeComposerMenuEvent('ContextMenu.onPerformCustomAction', [
        null,
        format['id'] as String,
      ]);
      await _nativeComposerMenuEvent('ContextMenu.onDismissSystemContextMenu');
      await tester.pumpAndSettle();
      expect(menus.last.map((item) => item['title']), [
        'Bold',
        'Italic',
        'Strikethrough',
        'Code',
        'Code block',
      ]);
      final action = menus.last.singleWhere(
        (item) => item['title'] == style.$1,
      );
      await _nativeComposerMenuEvent('ContextMenu.onPerformCustomAction', [
        null,
        action['id'] as String,
      ]);
      await tester.pumpAndSettle();
      expect(controller.text, style.$2);
      expect(
        tester.widget<TextField>(find.byType(TextField)).focusNode!.hasFocus,
        isTrue,
      );
    }
    controller.selection = TextSelection.collapsed(
      offset: controller.text.length,
    );
    editable.showToolbar();
    await tester.pumpAndSettle();
    expect(menus.last.where((item) => item['title'] == 'Format'), isEmpty);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('attachment popover stays in its resized chat pane', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1200, 900);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    addTearDown(tester.view.reset);
    final paneWidth = ValueNotifier(800.0);
    addTearDown(paneWidth.dispose);
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _TrayPickerService(),
        onSend: (_, _, {mediaTags = const []}) async {},
        wrapPage: (page) => ValueListenableBuilder<double>(
          valueListenable: paneWidth,
          builder: (_, width, _) => Align(
            alignment: Alignment.centerRight,
            child: SizedBox(width: width, child: page),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Message'));
    await tester.pumpAndSettle();
    final plus = find.byKey(const ValueKey('ios-composer-plus'));
    final glass = find.byWidgetPredicate(
      (widget) =>
          widget is UiKitView &&
          widget.viewType == 'buzz/composer_tool_strip_glass',
    );
    for (final width in [800.0, 500.0]) {
      paneWidth.value = width;
      await tester.pumpAndSettle();
      expect(tester.getRect(plus).left, 1200 - width + 12);
      await _tapComposerControl(tester, 'Attachments');
      await tester.pumpAndSettle();
      expect(glass, findsOneWidget);
      expect(tester.getRect(plus).right, lessThanOrEqualTo(1188));
      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
    }
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets(
    'iOS attachments and empty composer actions work without a keyboard',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(800, 900);
      tester.view.viewPadding = const FakeViewPadding(bottom: 34);
      tester.view.padding = const FakeViewPadding(bottom: 34);
      addTearDown(tester.view.reset);
      final pickerService = _TrayPickerService();
      const photoChannel = MethodChannel('buzz/inline_photo_picker');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        photoChannel,
        (call) async => true,
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          photoChannel,
          null,
        ),
      );
      const emojiChannel = MethodChannel('buzz/native_emoji_picker');
      final nativeCalls = <String>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        emojiChannel,
        (call) async {
          nativeCalls.add(call.method);
          return true;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          emojiChannel,
          null,
        ),
      );
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: pickerService,
          customEmoji: const [
            CustomEmoji(
              shortcode: 'party',
              url: 'https://example.org/party.png',
            ),
          ],
          onSend: (content, mentions, {mediaTags = const []}) async {},
        ),
      );
      expect(find.byTooltip('Add attachment'), findsOneWidget);
      await tester.tap(find.text('Message'));
      await tester.pumpAndSettle();
      for (final label in ['Emoji', 'Dictate']) {
        expect(find.byTooltip(label), findsOneWidget);
      }
      expect(find.byKey(const ValueKey('standard-actions')), findsNothing);
      expect(find.byTooltip('Add attachment'), findsOneWidget);
      await _tapComposerControl(tester, 'Attachments');
      await tester.pumpAndSettle();
      for (final label in ['camera', 'photos', 'voice note', 'files']) {
        expect(
          find.byKey(ValueKey('attachment-menu-item-$label')),
          findsOneWidget,
        );
      }
      expect(find.text('Dictate'), findsNothing);
      await tester.tap(
        find.byKey(const ValueKey('attachment-menu-item-camera')),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 200));
      expect(find.byKey(const ValueKey('camera-preview')), findsOneWidget);
      await tester.tap(find.byType(TextField));
      await tester.pumpAndSettle();
      await _selectComposerAttachment(tester, 'Files');
      await _selectComposerAttachment(tester, 'Photos');
      expect(pickerService.photoOpens, 0);
      expect(find.byKey(const ValueKey('photo-gallery')), findsOneWidget);
      expect(find.byKey(const ValueKey('attachment-menu')), findsNothing);
      expect(
        find.byKey(const ValueKey('ios-inline-photo-picker')),
        findsOneWidget,
      );
      expect(find.byTooltip('Keyboard'), findsNothing);
      await tester.tapAt(const Offset(20, 100));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Emoji'));
      await tester.pumpAndSettle();
      expect(nativeCalls, contains('present'));
      expect(tester.testTextInput.isVisible, isFalse);
      for (var index = 0; index < 2; index++) {
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          emojiChannel.name,
          emojiChannel.codec.encodeMethodCall(
            const MethodCall('selected', ':party:'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
        expect(tester.testTextInput.isVisible, isFalse);
      }
      expect(
        tester.widget<TextField>(find.byType(TextField).first).controller!.text,
        ':party::party:',
      );
      expect(find.byType(EmojiPickerSheet), findsNothing);
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        emojiChannel.name,
        emojiChannel.codec.encodeMethodCall(const MethodCall('dismissed')),
        (_) {},
      );
      await tester.pumpAndSettle();
      expect(tester.testTextInput.isVisible, isTrue);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets(
    'tapping the draft with the attachment menu open goes back to typing',
    (tester) async {
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _TrayPickerService(),
          onSend: (_, _, {mediaTags = const []}) async {},
        ),
      );
      await _tapComposerControl(tester, 'Attachments');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
      await tester.tap(find.text('Message'));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('attachment-menu')), findsNothing);
      expect(find.byType(TextField), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('choosing photos without the native popover closes the tray', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 900);
    addTearDown(tester.view.reset);
    const photoChannel = MethodChannel('buzz/inline_photo_picker');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      photoChannel,
      (call) async => false,
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        photoChannel,
        null,
      ),
    );
    Future<void> settle() async {
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
    }

    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        photoLibrary: _FakePhotoLibrary([
          RecentPhoto(id: 'one', thumbnailBytes: _gifBytes),
        ]),
        onSend: (content, mentions, {mediaTags = const []}) async {},
      ),
    );
    await _tapComposerControl(tester, 'Attachments');
    await settle();
    expect(find.byKey(const ValueKey('attachment-menu')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('attachment-menu-item-photos')));
    await settle();
    await tester.tap(find.byKey(const ValueKey('recent-photo-one')));
    await settle();
    await tester.tap(find.text('Add 1 photo'));
    await settle();
    expect(
      find.byWidgetPredicate(
        (w) => w.runtimeType.toString() == '_ComposerToolStripGlass',
      ),
      findsNothing,
    );
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('typed mentions and channels preserve the draft and keyboard', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 900);
    tester.view.viewPadding = const FakeViewPadding(bottom: 34);
    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      _buildComposeBar(
        channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
        members: [
          ChannelMember(
            pubkey: 'a' * 64,
            displayName: 'Scout',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        onSend: (content, mentions, {mediaTags = const []}) async {},
      ),
    );
    await tester.tap(find.text('Message'));
    await tester.pumpAndSettle();
    for (final action in [
      ('Mention', 'mention-suggestions-popover'),
      ('Channel', 'channel-suggestions-popover'),
    ]) {
      await tester.enterText(find.byType(TextField), '');
      await tester.pumpAndSettle();
      tester.testTextInput.log.clear();
      await tester.enterText(
        find.byType(TextField),
        action.$1 == 'Mention' ? '@' : '#',
      );
      await tester.pumpAndSettle();
      expect(tester.testTextInput.isVisible, isTrue);
      expect(
        tester.testTextInput.log.where(
          (call) => call.method == 'TextInput.hide',
        ),
        isEmpty,
      );
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        action.$1 == 'Mention' ? '@' : '#',
      );
      expect(find.byKey(ValueKey(action.$2)), findsOneWidget);
      expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
      await tester.tapAt(const Offset(20, 100));
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey(action.$2)), findsNothing);
      expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
      await tester.enterText(find.byType(TextField), '');
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byType(TextField),
        action.$1 == 'Mention' ? '@' : '#',
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(action.$1 == 'Mention' ? 'Scout' : 'shared'));
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey(action.$2)), findsNothing);
      expect(find.byKey(const ValueKey('ios-composer-plus')), findsOneWidget);
      expect(tester.testTextInput.isVisible, isTrue);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        action.$1 == 'Mention' ? '@Scout ' : '#shared ',
      );
    }
    // A real tap on Emoji while autocomplete is open opens the picker
    // instead of only dismissing the suggestions.
    const emojiChannel = MethodChannel('buzz/native_emoji_picker');
    var emojiPresentations = 0;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      emojiChannel,
      (call) async {
        if (call.method == 'present') emojiPresentations++;
        return true;
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        emojiChannel,
        null,
      ),
    );
    await tester.enterText(find.byType(TextField), '@');
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('mention-suggestions-popover')),
      findsOneWidget,
    );
    await tester.tap(find.byTooltip('Emoji'));
    await tester.pumpAndSettle();
    expect(emojiPresentations, 1);
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      emojiChannel.name,
      emojiChannel.codec.encodeMethodCall(const MethodCall('dismissed')),
      (_) {},
    );
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'Keep this draft');
    await tester.pumpAndSettle();
    final originalDraftTop = tester.getTopLeft(find.byType(TextField)).dy;
    await _tapComposerControl(tester, 'Attachments');
    await tester.pumpAndSettle();
    // This fixture declines native presentation, so the inline fallback
    // replaces the keyboard while preserving the draft and its position.
    expect(tester.testTextInput.isVisible, isFalse);
    expect(tester.getTopLeft(find.byType(TextField)).dy, originalDraftTop);
    await tester.tapAt(const Offset(20, 100));
    await tester.pumpAndSettle();
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      'Keep this draft',
    );
    expect(find.byKey(const ValueKey('attachment-surface')), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('a Send tap is not taken as a tap outside the suggestions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildComposeBar(
        channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
        members: [
          ChannelMember(
            pubkey: 'a' * 64,
            displayName: 'Scout',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        onSend: (content, mentions, {mediaTags = const []}) async {},
      ),
    );
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), 'hi @Sc');
    await tester.pumpAndSettle();
    final suggestions = find.byKey(
      const ValueKey('mention-suggestions-popover'),
    );
    expect(suggestions, findsOneWidget);
    // Send is a native view, so a widget test cannot press it. A tap the
    // tray consumes as "outside" dismisses the suggestions and never reaches
    // the native button; a tap that reaches it leaves them in place.
    await tester.tap(find.byTooltip('Send'));
    await tester.pumpAndSettle();
    expect(suggestions, findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
