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
import 'package:flutter/semantics.dart';
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
  bool failRead = false;
  @override
  Future<VoiceNoteRecording?> read() async {
    if (failRead) throw const FormatException('Truncated draft');
    return note;
  }

  @override
  Future<VoiceNoteRecording> save(VoiceNoteRecording value) async {
    if (failSave) throw FileSystemException('No space left on device');
    note = value;
    return value;
  }

  @override
  Future<void> clear({String? generation}) async {
    note = null;
    failRead = false;
  }
}

class ObservedDiskDraft extends DefaultAgentVoiceDraft {
  ObservedDiskDraft(Directory directory)
    : super('test', directory: () async => directory);
  final cleared = Completer<void>();
  @override
  Future<void> clear({String? generation}) async {
    await super.clear(generation: generation);
    if (!cleared.isCompleted) cleared.complete();
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
  Future<void> mount(
    WidgetTester tester, {
    bool nativeAvatar = false,
    Axis axis = Axis.horizontal,
    DefaultAgentVoiceDraft? storage,
    Future<void> Function(VoiceNoteRecording)? onRecorded,
  }) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          voiceNoteRecorderFactoryProvider.overrideWithValue(() => recorder),
          defaultAgentVoiceDraftProvider(
            'test',
          ).overrideWithValue(storage ?? draft),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: MessagesNavigationBar(
                axis: axis,
                onSearch: () {},
                onRecordingChanged: recordingStates.add,
                creationActions: const [],
                agent: DefaultAgentVoiceControl(
                  enabled: true,
                  draftIdentity: 'test',
                  onRecorded:
                      onRecorded ??
                      (_) async {
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
    final surface = find.byWidgetPredicate(
      (widget) =>
          widget is UiKitView && widget.viewType == 'buzz/navigation_glass',
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
    // Shared native glass is hidden during delivery and recovery. The expanded
    // control must retain its own visible surface and its left-side hit targets.
    recorder = Recorder();
    fail = true;
    final retryGesture = await hold(tester);
    await retryGesture.up();
    await tester.pumpAndSettle();
    final recoverySurface = tester.widget<DecoratedBox>(
      find.byKey(const ValueKey('default-agent-voice-surface')),
    );
    expect(
      (recoverySurface.decoration as BoxDecoration).color,
      tester.element(find.byType(DefaultAgentVoiceControl)).controlSurface,
    );
    expect(tester.element(surface.first), same(touchedSurface));
    await mount(tester, nativeAvatar: true, axis: Axis.vertical);
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Discard voice note'));
    await tester.pumpAndSettle();
    expect(draft.note, isNull);
    expect(find.text('Retry voice note'), findsNothing);
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
    final semantics = tester.ensureSemantics();
    void expectRecoverySemantics(String value) {
      final node = tester.getSemantics(find.byType(DefaultAgentVoiceControl));
      expect(node.value, value);
      expect(node.hint, isNot(contains('Hold to record')));
      expect(
        node.getSemanticsData().hasAction(SemanticsAction.longPress),
        isFalse,
      );
      final surface = tester.widget<Semantics>(
        find
            .descendant(
              of: find.byType(DefaultAgentVoiceControl),
              matching: find.byType(Semantics),
            )
            .first,
      );
      expect(surface.properties.customSemanticsActions, isNull);
      final discard = tester.getSemantics(find.byTooltip('Discard voice note'));
      expect(discard.tooltip, 'Discard voice note');
      expect(discard.getSemanticsData().hasAction(SemanticsAction.tap), isTrue);
    }

    fail = true;
    await mount(tester);
    final gesture = await hold(tester);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(draft.note, isNotNull);
    expectRecoverySemantics('Voice note awaiting delivery');
    await tester.tap(find.byTooltip('Discard voice note'));
    await tester.pumpAndSettle();
    expect(draft.note, isNull);
    expect(sent, 0);
    expect(find.byTooltip('Agent'), findsOneWidget);
    // An unreadable restored draft still exposes Discard and can unblock UI.
    await tester.pumpWidget(const SizedBox.shrink());
    draft.failRead = true;
    await mount(tester);
    await tester.pumpAndSettle();
    expect(find.text('Could not restore voice note. Retry'), findsOneWidget);
    expectRecoverySemantics('Could not restore voice note');
    await tester.tap(find.byTooltip('Discard voice note'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Agent'), findsOneWidget);
    expect(find.text('Could not restore voice note. Retry'), findsNothing);
    semantics.dispose();
  });
  testWidgets('late delivery cannot clear a replacement after remount', (
    tester,
  ) async {
    late Directory directory;
    late ObservedDiskDraft storage;
    late VoiceNoteRecording replacement;
    await tester.runAsync(() async {
      directory = await Directory.systemTemp.createTemp('voice-remount');
      storage = ObservedDiskDraft(directory);
      final source = File('${directory.path}/first.m4a');
      await source.writeAsBytes([1, 2, 3]);
      await storage.save(
        VoiceNoteRecording(
          file: XFile(source.path),
          duration: const Duration(seconds: 2),
          waveform: const [.2],
        ),
      );
    });
    addTearDown(() => directory.delete(recursive: true));
    late Completer<void> acknowledged;
    late Completer<void> sendingStarted;
    await tester.runAsync(() async {
      acknowledged = Completer<void>();
      sendingStarted = Completer<void>();
      await mount(
        tester,
        storage: storage,
        onRecorded: (_) {
          sendingStarted.complete();
          return acknowledged.future;
        },
      );
      // Drain the real storage read before interacting with the recovery UI.
      await storage.read();
    });
    await tester.pumpAndSettle();
    await tester.runAsync(() async {
      await tester.tap(find.text('Retry voice note'));
      await sendingStarted.future;
    });
    await tester.pumpWidget(const SizedBox.shrink());
    final remounted = DefaultAgentVoiceDraft(
      'test',
      directory: () async => directory,
    );
    await tester.runAsync(() async {
      await mount(tester, storage: remounted);
      await remounted.read();
    });
    await tester.pumpAndSettle();
    await tester.runAsync(() async {
      await tester.tap(find.byTooltip('Discard voice note'));
      // This mutation queues behind the UI's explicit discard.
      final source = File('${directory.path}/replacement.m4a');
      await source.writeAsBytes([4, 5, 6]);
      replacement = await remounted.save(
        VoiceNoteRecording(
          file: XFile(source.path),
          duration: const Duration(seconds: 3),
          waveform: const [.8],
        ),
      );
      acknowledged.complete();
      await storage.cleared.future;
    });
    await tester.pumpAndSettle();
    await tester.runAsync(() async {
      final recovered = await remounted.read();
      expect(recovered, isNotNull);
      expect(recovered!.draftGeneration, replacement.draftGeneration);
      expect(await recovered.file.readAsBytes(), [4, 5, 6]);
    });
    expect(tester.takeException(), isNull);
  });
}
