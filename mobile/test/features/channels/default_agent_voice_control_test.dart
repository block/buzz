import 'dart:async';
import 'dart:io';

import 'package:buzz/features/channels/default_agent_voice_control.dart';
import 'package:buzz/features/channels/default_agent_voice_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/features/channels/voice_note_waveform.dart';
import 'package:buzz/features/home/messages_navigation_bar.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';

class Recorder implements VoiceNoteRecorder {
  late final stream = StreamController<double>.broadcast(sync: true);
  Completer<void>? startup;
  bool cancelled = false;
  int stops = 0;
  @override
  Stream<double> get levels => stream.stream;
  @override
  Future<void> start() async {
    await startup?.future;
  }

  @override
  Future<VoiceNoteRecording> stop() async {
    stops++;
    return VoiceNoteRecording(
      file: XFile('/tmp/agent-voice-test.m4a'),
      duration: const Duration(seconds: 2),
      waveform: const [.2, .8],
    );
  }

  @override
  Future<void> cancel() async {
    cancelled = true;
  }

  @override
  Future<void> dispose() async {
    await stream.close();
  }
}

class Draft extends DefaultAgentVoiceDraft {
  Draft() : super('test');
  VoiceNoteRecording? note;
  bool failSave = false;
  @override
  Future<VoiceNoteRecording?> read() async => note;
  @override
  Future<VoiceNoteRecording> save(VoiceNoteRecording value) async {
    if (failSave) throw FileSystemException('No space left on device');
    note = value;
    return value;
  }

  @override
  Future<void> clear() async {
    note = null;
  }
}

void main() {
  late Recorder recorder;
  late Draft draft;
  late int sent;
  late int taps;
  late bool fail;
  Completer<void>? delivery;
  final haptics = <String>[];
  final recordingStates = <bool>[];
  setUp(() {
    recorder = Recorder();
    draft = Draft();
    sent = 0;
    taps = 0;
    fail = false;
    delivery = null;
    haptics.clear();
    recordingStates.clear();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') {
            haptics.add(call.arguments as String);
          }
          return null;
        });
  });
  Future<void> mount(WidgetTester tester, {bool nativeAvatar = false}) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          voiceNoteRecorderFactoryProvider.overrideWithValue(() => recorder),
          defaultAgentVoiceDraftProvider('test').overrideWithValue(draft),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: MessagesNavigationBar(
                onSearch: () {},
                onRecordingChanged: recordingStates.add,
                creationActions: const [],
                agent: DefaultAgentVoiceControl(
                  enabled: true,
                  draftIdentity: 'test',
                  onRecorded: (_) async {
                    if (delivery != null) await delivery!.future;
                    if (fail) throw StateError('offline');
                    sent++;
                  },
                  child: SizedBox.square(
                    dimension: 56,
                    child: IconButton(
                      tooltip: 'Agent',
                      onPressed: () => taps++,
                      icon: nativeAvatar
                          ? const UiKitView(viewType: 'test/agent-video')
                          : const Icon(Icons.person),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
  }

  Future<TestGesture> hold(WidgetTester tester) async {
    final gesture = await tester.startGesture(
      tester.getCenter(find.byTooltip('Agent')),
    );
    await tester.pump(const Duration(milliseconds: 600));
    await tester.pump(const Duration(milliseconds: 250));
    return gesture;
  }

  testWidgets('iOS keeps the touched native surface through recording', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    final messenger = tester.binding.defaultBinaryMessenger;
    const support = MethodChannel('buzz/concentric_sheet_surface');
    final disposed = <int>[];
    final created = <int>[];
    messenger.setMockMethodCallHandler(support, (_) async => true);
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'dispose') disposed.add(call.arguments as int);
      if (call.method == 'create') {
        created.add((call.arguments as Map)['id'] as int);
      }
      return null;
    });
    addTearDown(() {
      debugDefaultTargetPlatformOverride = null;
      messenger.setMockMethodCallHandler(support, null);
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
    });
    await mount(tester, nativeAvatar: true);
    await tester.pumpAndSettle();
    final surface = find.descendant(
      of: find.byType(DefaultAgentVoiceControl),
      matching: find.byWidgetPredicate(
        (widget) =>
            widget is UiKitView &&
            widget.viewType == 'buzz/concentric_sheet_surface',
      ),
    );
    expect(surface, findsOneWidget);
    final touchedSurface = tester.element(surface.first);
    final avatar = find.byWidgetPredicate(
      (widget) => widget is UiKitView && widget.viewType == 'test/agent-video',
    );
    final touchedAvatar = tester.element(avatar);
    final retainedIds = created.toSet();
    final gesture = await hold(tester);
    expect(tester.element(surface.first), same(touchedSurface));
    expect(tester.element(avatar), same(touchedAvatar));
    expect(disposed.where(retainedIds.contains), isEmpty);
    expect(find.byType(VoiceNoteWaveform), findsOneWidget);
    await gesture.moveBy(const Offset(0, -90));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(tester.element(surface.first), same(touchedSurface));
    expect(tester.element(avatar), same(touchedAvatar));
    expect(disposed.where(retainedIds.contains), isEmpty);
    expect(recorder.cancelled, isTrue);
    expect(sent, 0);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pump();
    debugDefaultTargetPlatformOverride = null;
  });
  testWidgets(
    'tap opens chat; hold expands left with real levels and release sends once',
    (tester) async {
      await mount(tester);
      await tester.tap(find.byTooltip('Agent'));
      expect(taps, 1);
      final before = tester.getRect(
        find.byKey(const ValueKey('default-agent-voice-pill')),
      );
      final gesture = await hold(tester);
      recorder.stream.add(.8);
      await tester.pump();
      final after = tester.getRect(
        find.byKey(const ValueKey('default-agent-voice-pill')),
      );
      expect(after.right, before.right);
      expect(after.width, 256);
      expect(
        tester
            .getRect(find.byKey(const ValueKey('voice-cancel-target')))
            .bottom,
        lessThan(after.top),
      );
      expect(
        tester
            .widget<VoiceNoteWaveform>(find.byType(VoiceNoteWaveform))
            .samples
            .last,
        .8,
      );
      await gesture.up();
      await tester.pumpAndSettle();
      expect(sent, 1);
      expect(recorder.stops, 1);
      expect(draft.note, isNull);
      expect(taps, 1);
      expect(tester.takeException(), isNull);
    },
  );
  testWidgets('release while permission is pending cancels without sending', (
    tester,
  ) async {
    recorder.startup = Completer<void>();
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.up();
    await tester.pump();
    expect(haptics, ['HapticFeedbackType.mediumImpact']);
    recorder.startup!.complete();
    await tester.pumpAndSettle();
    expect(recorder.cancelled, isTrue);
    expect(recorder.stops, 0);
    expect(sent, 0);
  });
  testWidgets('iPhone prewarms feedback before hold and cancel thresholds', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const channel = MethodChannel('buzz/haptics');
    final calls = <String>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          calls.add(call.method);
          return null;
        });
    addTearDown(() {
      debugDefaultTargetPlatformOverride = null;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null);
    });
    await mount(tester);
    await tester.pumpAndSettle();
    final gesture = await tester.startGesture(
      tester.getCenter(find.byTooltip('Agent')),
    );
    await tester.pump();
    expect(calls, ['voicePrepareStart']);
    await tester.pump(const Duration(milliseconds: 600));
    expect(calls, contains('voiceStart'));
    await gesture.moveBy(const Offset(0, -30));
    await tester.pump();
    expect(calls.last, 'voicePrepareCancel');
    await gesture.moveBy(const Offset(0, -45));
    await tester.pump();
    expect(calls.last, 'voiceCancel');
    final count = calls.length;
    await gesture.moveBy(const Offset(0, -10));
    await tester.pump();
    expect(calls.length, count);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(recorder.cancelled, isTrue);
    expect(sent, 0);
    debugDefaultTargetPlatformOverride = null;
  });
  testWidgets('slide up cancels and never sends', (tester) async {
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.moveBy(const Offset(0, -90));
    await tester.pump();
    expect(
      tester.getSemantics(find.byType(DefaultAgentVoiceControl)).value,
      contains('Release to cancel'),
    );
    final target = find.byKey(const ValueKey('voice-cancel-target'));
    expect(tester.widget<AnimatedScale>(target).scale, 1.15);
    expect(recordingStates.last, isTrue);
    final targetContainer = tester.widget<AnimatedContainer>(
      find.descendant(of: target, matching: find.byType(AnimatedContainer)),
    );
    expect(
      (targetContainer.decoration! as BoxDecoration).color,
      AppTheme.light().colorScheme.error,
    );
    expect(haptics, [
      'HapticFeedbackType.mediumImpact',
      'HapticFeedbackType.selectionClick',
    ]);
    await gesture.moveBy(const Offset(0, -10));
    await tester.pump();
    expect(haptics.length, 2);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(recorder.cancelled, isTrue);
    expect(sent, 0);
    expect(recordingStates.last, isFalse);
  });
  testWidgets('release shows the avatar loader until delivery finishes', (
    tester,
  ) async {
    delivery = Completer<void>();
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.up();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 300));
    expect(find.byType(BuzzLoadingIndicator), findsOneWidget);
    expect(sent, 0);
    delivery!.complete();
    await tester.pumpAndSettle();
    expect(sent, 1);
    expect(find.byType(BuzzLoadingIndicator), findsNothing);
  });
  testWidgets('save failure stays unsaved; delivery failure retains retry', (
    tester,
  ) async {
    draft.failSave = true;
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(draft.note, isNull);
    expect(sent, 0);
    expect(
      find.textContaining('Could not save your voice note.'),
      findsOneWidget,
    );
    expect(find.textContaining('Your voice note is saved.'), findsNothing);
    ScaffoldMessenger.of(
      tester.element(find.byType(DefaultAgentVoiceControl)),
    ).hideCurrentSnackBar();
    await tester.pumpAndSettle();
    draft.failSave = false;
    fail = true;
    await tester.tap(find.text('Retry saving voice note'));
    await tester.pumpAndSettle();
    expect(draft.note, isNotNull);
    expect(sent, 0);
    expect(find.textContaining('Your voice note is saved.'), findsOneWidget);
    fail = false;
    await tester.tap(find.text('Retry voice note'));
    await tester.pumpAndSettle();
    expect(sent, 1);
    expect(draft.note, isNull);
  });
  testWidgets('discarding a failed send restores the navigation shortcuts', (
    tester,
  ) async {
    fail = true;
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(draft.note, isNotNull);
    await tester.tap(find.byTooltip('Discard voice note'));
    await tester.pumpAndSettle();
    expect(draft.note, isNull);
    expect(sent, 0);
    expect(find.byTooltip('Agent'), findsOneWidget);
  });
}
