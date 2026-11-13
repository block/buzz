part of '../compose_bar_test.dart';

const _dictationCapability = TranscriptionCapability(
  path: '/audio/transcriptions',
);

final _dictatedAudio = Uint8List.fromList(utf8.encode('fake m4a audio'));

/// A recorder whose take is held in memory, like a finished m4a file.
class _DictationRecorder extends _FakeVoiceNoteRecorder {
  _DictationRecorder({this.denyMicrophone = false, this.unreadable = false});

  final bool denyMicrophone;
  final bool unreadable;

  @override
  Future<void> start() async {
    if (denyMicrophone) {
      throw StateError('Microphone access is required to record.');
    }
    await super.start();
  }

  @override
  Future<VoiceNoteRecording> stop() async => VoiceNoteRecording(
    file: unreadable
        ? _UnreadableRecording()
        : XFile.fromData(
            _dictatedAudio,
            path: '/tmp/dictation-test.m4a',
            mimeType: 'audio/mp4',
          ),
    duration: const Duration(seconds: 3),
    waveform: const [0.4, 0.8],
  );
}

/// A finished recording whose file is already gone.
class _UnreadableRecording extends XFile {
  _UnreadableRecording() : super('/tmp/dictation-test-gone.m4a');

  @override
  Future<Uint8List> readAsBytes() =>
      Future.error(const FileSystemException('gone'));
}

/// The relay's HTTP boundary: each request waits for the test to answer it.
/// Closing a request's client aborts it, as a real client does, unless
/// [ignoresClose] keeps it running so a late answer still arrives.
class _TranscriptionRelay {
  _TranscriptionRelay({this.ignoresClose = false});

  final bool ignoresClose;
  final requests = <http.Request>[];
  final _answers = <Completer<http.Response>>[];

  /// Requests whose client closed before they were answered.
  var aborted = 0;

  http.Client client() {
    Completer<http.Response>? answer;
    return _ClosableClient(
      http_testing.MockClient((request) {
        requests.add(request);
        _answers.add(answer = Completer<http.Response>());
        return answer!.future;
      }),
      onClose: () {
        if (answer == null || answer!.isCompleted) return;
        aborted++;
        if (!ignoresClose) {
          answer!.completeError(http.ClientException('client closed'));
        }
      },
    );
  }

  void answerText(String text) =>
      _answers.last.complete(http.Response(jsonEncode({'text': text}), 200));

  void answerStatus(int status) =>
      _answers.last.complete(http.Response('{"error":"x"}', status));

  void dropConnection() =>
      _answers.last.completeError(http.ClientException('connection reset'));
}

class _ClosableClient extends http.BaseClient {
  _ClosableClient(this._inner, {required this.onClose});

  final http.Client _inner;
  final VoidCallback onClose;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      _inner.send(request);

  @override
  void close() => onClose();
}

/// A composer on a relay that advertises transcription, unless
/// [transcription] says otherwise.
Widget _dictationBar({
  _TranscriptionRelay? relay,
  VoiceNoteRecorder Function()? recorder,
  TranscriptionCapability? transcription = _dictationCapability,
  String? threadHeadId,
  VoidCallback? onSend,
  DateTime Function()? clock,
}) => _buildComposeBar(
  uploadService: _testUploadService(nostr.Keys.generate().nsec),
  voiceNoteRecorderFactory: recorder ?? _DictationRecorder.new,
  voiceNoteClock: clock,
  transcription: transcription,
  relayHttpClientFactory: (relay ?? _TranscriptionRelay()).client,
  threadHeadId: threadHeadId,
  onSend: (content, mentions, {mediaTags = const <List<String>>[]}) async =>
      onSend?.call(),
);

/// Whether Send can be pressed. The collapsed and expanded rows each hold
/// one Send button, so every one in the tree must agree.
bool _sendEnabled(WidgetTester tester) {
  final enabled = {
    for (final button in tester.widgetList<IconButton>(
      find.ancestor(
        of: find.byIcon(BuzzIcons.arrowUp),
        matching: find.byType(IconButton),
      ),
    ))
      button.onPressed != null,
  };
  expect(enabled, hasLength(1), reason: 'Send buttons disagree');
  return enabled.single;
}

Future<void> _dictate(WidgetTester tester) async {
  await tester.tap(find.byTooltip('Dictate'));
  await tester.pumpAndSettle();
  expect(find.byTooltip('Stop and transcribe'), findsOneWidget);
  expect(find.byTooltip('Cancel dictation'), findsOneWidget);
  await tester.tap(find.byKey(const ValueKey('voice-note-recorder-stop')));
  await tester.pump();
  // Let the recorder row morph back into the composer (140 ms) without
  // settling, since the transcribing spinner never stops.
  await tester.pump(const Duration(milliseconds: 300));
}

/// Lets the transcript reach the composer without settling the spinner.
Future<void> _settleTranscript(WidgetTester tester) async {
  for (var i = 0; i < 5; i++) {
    await tester.pump(const Duration(milliseconds: 50));
  }
  await tester.pump(const Duration(milliseconds: 300));
}

List<String> _captureAnnouncements(WidgetTester tester) {
  final announcements = <String>[];
  tester.binding.defaultBinaryMessenger.setMockDecodedMessageHandler<dynamic>(
    SystemChannels.accessibility,
    (message) async {
      final event = message as Map<Object?, Object?>;
      if (event['type'] == 'announce') {
        announcements.add((event['data']! as Map)['message']! as String);
      }
      return null;
    },
  );
  addTearDown(
    () => tester.binding.defaultBinaryMessenger
        .setMockDecodedMessageHandler<dynamic>(
          SystemChannels.accessibility,
          null,
        ),
  );
  return announcements;
}

void dictationTests() {
  group('dictation', () {
    testWidgets('is hidden when the relay does not advertise transcription', (
      tester,
    ) async {
      await tester.pumpWidget(_dictationBar(transcription: null));
      await tester.pumpAndSettle();
      expect(find.byTooltip('Dictate'), findsNothing);
      await _expandComposer(tester);
      expect(find.byTooltip('Dictate'), findsNothing);
    });

    testWidgets(
      'dictates into the draft, survives failures and never sends by itself',
      (tester) async {
        final relay = _TranscriptionRelay();
        final announcements = _captureAnnouncements(tester);
        var sends = 0;
        var denyMicrophone = false;
        var unreadable = false;
        var now = DateTime.utc(2026, 11, 13);
        await tester.pumpWidget(
          _dictationBar(
            relay: relay,
            recorder: () => _DictationRecorder(
              denyMicrophone: denyMicrophone,
              unreadable: unreadable,
            ),
            onSend: () => sends++,
            clock: () => now,
          ),
        );
        await tester.pumpAndSettle();

        await _expandComposer(tester);
        await tester.enterText(find.byType(TextField), 'Keep this draft');
        await tester.pump();

        // Like a voice note, recording waits for the keyboard to go away.
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        addTearDown(tester.view.reset);
        await tester.pump();
        await tester.tap(find.byTooltip('Dictate'));
        await tester.pump();
        expect(find.byTooltip('Stop and transcribe'), findsNothing);
        expect(find.byTooltip('Dictate'), findsNothing);
        tester.view.viewInsets = FakeViewPadding.zero;
        await tester.pumpAndSettle();
        expect(find.byTooltip('Stop and transcribe'), findsOneWidget);
        await tester.tap(find.byTooltip('Cancel dictation'));
        await tester.pumpAndSettle();
        expect(find.text('Keep this draft'), findsOneWidget);

        // Recording replaces the composer; transcribing fences Send.
        await _dictate(tester);
        expect(find.byTooltip('Cancel transcription'), findsOneWidget);
        expect(find.byTooltip('Dictate'), findsNothing);
        expect(_sendEnabled(tester), isFalse);
        expect(
          announcements,
          containsAllInOrder(['Dictation recording', 'Transcribing']),
        );

        // The upload is the raw recording, NIP-98-signed over its own bytes.
        final request = relay.requests.single;
        expect(request.method, 'POST');
        expect(
          request.url.toString(),
          'http://localhost:3000/audio/transcriptions',
        );
        expect(request.headers['Content-Type'], 'audio/mp4');
        expect(request.headers['Authorization'], startsWith('Nostr '));
        expect(request.bodyBytes, _dictatedAudio);

        // The microphone stays with dictation until its transcript lands.
        await tester.tap(find.byTooltip('Add attachment').hitTestable());
        await tester.pump(const Duration(milliseconds: 500));
        await tester.tap(find.text('Voice note'));
        await tester.pump(const Duration(milliseconds: 500));
        expect(
          find.text('Finish dictating before recording a voice note.'),
          findsOneWidget,
        );
        expect(find.byTooltip('Stop recording'), findsNothing);

        relay.answerText('  dictated words ');
        await _settleTranscript(tester);
        expect(find.text('Keep this draft dictated words'), findsOneWidget);
        expect(find.byTooltip('Dictate'), findsOneWidget);
        expect(_sendEnabled(tester), isTrue);
        expect(sends, 0);
        expect(announcements.last, 'Transcript added');

        // Failures keep the draft, say what happened, and leave dictation on.
        const serviceFailure =
            'Transcription failed. Nothing was added; try dictating again.';
        for (final (fail, message) in [
          (
            relay.dropConnection,
            'Couldn’t reach the server. Nothing was added; try dictating again.',
          ),
          (() => relay.answerStatus(502), serviceFailure),
          (
            () => relay.answerStatus(429),
            'Too many dictations. Try again in a minute.',
          ),
          (
            () => relay.answerStatus(413),
            'This recording is too long to transcribe. Try a shorter one.',
          ),
          (
            () => relay.answerText('   '),
            'No speech detected. Nothing was added.',
          ),
        ]) {
          await _dictate(tester);
          fail();
          await _settleTranscript(tester);
          expect(find.text(message), findsOneWidget);
          expect(announcements.last, message);
          expect(find.text('Keep this draft dictated words'), findsOneWidget);
          expect(find.byTooltip('Dictate'), findsOneWidget);
          expect(_sendEnabled(tester), isTrue);
        }

        // A recording that can't be read fails the same way, with no upload.
        unreadable = true;
        await _dictate(tester);
        await _settleTranscript(tester);
        expect(find.text(serviceFailure), findsOneWidget);
        expect(find.text('Keep this draft dictated words'), findsOneWidget);
        unreadable = false;

        // Cancelling a transcription aborts its request, quietly.
        await _dictate(tester);
        await tester.tap(find.byTooltip('Cancel transcription'));
        await tester.pump();
        expect(relay.aborted, 1);
        expect(find.byTooltip('Dictate'), findsOneWidget);
        expect(_sendEnabled(tester), isTrue);
        await _settleTranscript(tester);
        expect(find.textContaining('Couldn’t reach'), findsNothing);
        expect(find.text('Keep this draft dictated words'), findsOneWidget);

        // A denied microphone says so and keeps the draft.
        denyMicrophone = true;
        await tester.tap(find.byTooltip('Dictate'));
        await tester.pumpAndSettle();
        expect(
          find.text('Microphone access is required to record.'),
          findsOneWidget,
        );
        await tester.tap(find.byTooltip('Cancel dictation'));
        await tester.pumpAndSettle();
        expect(find.text('Keep this draft dictated words'), findsOneWidget);
        denyMicrophone = false;

        // The recorder's own 5-minute auto-stop transcribes like Stop does.
        await tester.tap(find.byTooltip('Dictate'));
        await tester.pumpAndSettle();
        expect(find.byTooltip('Stop and transcribe'), findsOneWidget);
        now = now.add(voiceNoteMaxDuration);
        await tester.pump(const Duration(milliseconds: 200));
        await tester.pump(const Duration(milliseconds: 300));
        expect(find.byTooltip('Cancel transcription'), findsOneWidget);
        relay.answerText('the whole take');
        await _settleTranscript(tester);
        expect(
          find.text('Keep this draft dictated words the whole take'),
          findsOneWidget,
        );
        expect(relay.requests, hasLength(8));
        expect(sends, 0);

        // A voice note holds the microphone from the keyboard wait until it
        // is removed, so Dictate stays hidden while it is pending or attached.
        await _openAttachmentMenu(tester);
        tester.view.viewInsets = const FakeViewPadding(bottom: 300);
        await tester.pump();
        await tester.tap(find.text('Voice note'));
        await tester.pump();
        expect(find.byTooltip('Stop recording'), findsNothing);
        expect(find.byTooltip('Dictate'), findsNothing);
        tester.view.viewInsets = FakeViewPadding.zero;
        await tester.pumpAndSettle();
        await tester.tap(
          find.byKey(const ValueKey('voice-note-recorder-stop')),
        );
        await tester.pumpAndSettle();
        expect(find.byTooltip('Remove voice note'), findsOneWidget);
        expect(find.byTooltip('Dictate'), findsNothing);
        await tester.tap(find.byTooltip('Remove voice note'));
        await tester.pumpAndSettle();
        expect(find.byTooltip('Dictate'), findsOneWidget);
      },
    );

    testWidgets('a late transcript never lands in another thread’s draft', (
      tester,
    ) async {
      // Even if the abort is ignored, the generation fence drops the answer.
      final relay = _TranscriptionRelay(ignoresClose: true);
      await tester.pumpWidget(_dictationBar(relay: relay));
      await tester.pumpAndSettle();
      await _dictate(tester);

      await tester.pumpWidget(
        _dictationBar(relay: relay, threadHeadId: 'thread-2'),
      );
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.byTooltip('Dictate'), findsOneWidget);
      expect(relay.aborted, 1);
      relay.answerText('meant for the channel');
      await _settleTranscript(tester);
      expect(find.textContaining('meant for the channel'), findsNothing);
      expect(find.text('Message…'), findsOneWidget);
    });
  });
}
