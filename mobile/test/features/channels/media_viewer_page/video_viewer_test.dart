// Regression tests for MediaVideoViewerPage lifecycle fixes:
//
// F2r(a): VideoPlayerController is disposed when native initialisation fails.
//         Before the fix, a PlatformException from initialize() unwound to the
//         outer catch with controller.value still null — the cleanup teardown's
//         `if (activeController != null)` guard silently skipped disposal,
//         leaking the native player and its event subscription.
//
// F2r(b): A non-2xx error-body is cancelled (listen+cancel) rather than
//         drained.  Before the fix, `response.stream.drain()` waited for the
//         server to close the stream — a stalled error body (e.g. a 403 on
//         a slow connection) could block initializeVideo indefinitely, and
//         there was no abort handle to cancel it.
//
// F2r(c): A VideoPlayerController created but never initialized (pending)
//         is disposed when the widget is unmounted.  Before the fix, the
//         controller lived only in the async function's stack frame; a close
//         arriving while initialize() was awaiting the initialized event left
//         the native player allocated forever.
//
// F2r(d): createWithOptions() failure shows the error UI instead of leaving
//         the viewer in an infinite loading state.  video_player 2.11.1
//         creates _creatingCompleter before awaiting createWithOptions() and
//         completes it only AFTER the await returns.  If creation throws,
//         _creatingCompleter is never completed and dispose() deadlocks waiting
//         on it.  The fix uses unawaited(dispose()) in the catch so the outer
//         catch runs immediately and sets error.value.
//
// Transport: AbortableStreamedRequest sink must be closed before send().
//         Without it, IOClient.send() awaits stream.pipe(ioRequest) which
//         blocks until the sink is closed — every download hangs indefinitely
//         with the real http.Client.  Two probes: a local loopback server that
//         reads the full request body before replying (fails if sink is not
//         closed), and a MockClient-based fake that calls request.finalize()
//         for unit coverage.

import 'dart:async';
import 'dart:io';

import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:path_provider_platform_interface/path_provider_platform_interface.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';
import 'package:video_player_platform_interface/video_player_platform_interface.dart';

import '../../../helpers/widget_helpers.dart';

// ── Fakes ────────────────────────────────────────────────────────────────────

/// Minimal fake for path_provider so getTemporaryDirectory() works in tests.
/// Uses MockPlatformInterfaceMixin to bypass PlatformInterface.verify().
class _FakePathProviderPlatform extends Fake
    with MockPlatformInterfaceMixin
    implements PathProviderPlatform {
  @override
  Future<String?> getTemporaryPath() async =>
      Directory.systemTemp.resolveSymbolicLinksSync();
}

/// Fake VideoPlayerPlatform that tracks `dispose` calls and optionally
/// forces native initialisation to fail with a PlatformException.
///
/// Extends VideoPlayerPlatform directly (inheriting the platform token from the
/// super constructor) so PlatformInterface.verify() succeeds without needing
/// MockPlatformInterfaceMixin.
class _FakeVideoPlayerPlatform extends VideoPlayerPlatform {
  final bool forceInitError;
  final bool neverInitialize;
  // When true, createWithOptions() itself throws a PlatformException.
  // video_player 2.11.1 awaits createWithOptions() before completing
  // _creatingCompleter (video_player.dart:587-590); if creation throws,
  // _creatingCompleter is never completed and dispose() deadlocks waiting
  // on it.  This flag exercises the F2r(d) production fix.
  final bool forceCreateError;
  int disposeCallCount = 0;
  int nextPlayerId = 0;
  final Map<int, StreamController<VideoEvent>> _streams = {};

  _FakeVideoPlayerPlatform({
    this.forceInitError = false,
    this.neverInitialize = false,
    this.forceCreateError = false,
  });

  @override
  Future<void> init() async {}

  @override
  Future<int?> createWithOptions(VideoCreationOptions options) async {
    if (forceCreateError) {
      throw PlatformException(
        code: 'VideoError',
        message: 'Fake native create failure',
      );
    }
    return create(options.dataSource);
  }

  @override
  Future<int?> create(DataSource dataSource) async {
    final id = nextPlayerId++;
    final controller = StreamController<VideoEvent>(
      onListen: () {
        // Emit the event/error only when the stream is first subscribed so that
        // VideoPlayerController.initialize() is already listening.  Emitting
        // before the subscription means the event is dropped and initialize()
        // hangs waiting for the initialized signal.
        if (forceInitError) {
          _streams[id]!.addError(
            PlatformException(
              code: 'VideoError',
              message: 'Fake native init failure',
            ),
          );
        } else if (!neverInitialize) {
          _streams[id]!.add(
            VideoEvent(
              eventType: VideoEventType.initialized,
              size: const Size(100, 100),
              duration: const Duration(seconds: 1),
            ),
          );
        }
        // neverInitialize: no event emitted — initialize() hangs forever.
      },
    );
    _streams[id] = controller;
    return id;
  }

  @override
  Future<void> dispose(int playerId) async {
    disposeCallCount++;
    // Record the dispose call and close the event stream.
    //
    // Note: an error injected here does NOT reach initialize()'s pending
    // listener on the pinned video_player 2.11.1 path.  dispose() cancels
    // _eventSubscription (:687) before calling _videoPlayerPlatform.dispose()
    // (:688), so any event emitted here goes to a closed listener.  This fake
    // records the disposal call and closes its stream; it does not settle the
    // pending initialize() future.
    final stream = _streams[playerId];
    if (stream != null) {
      if (!stream.isClosed) {
        stream.addError(
          StateError('VideoPlayerController disposed before initialization'),
        );
      }
      await stream.close();
    }
  }

  /// Return a minimal stand-in widget.  VideoPlayerPlatform.buildViewWithOptions
  /// delegates to this; without it every test that successfully initializes a
  /// player throws UnimplementedError when the VideoPlayer widget renders.
  @override
  Widget buildView(int playerId) => const SizedBox.shrink();

  @override
  Stream<VideoEvent> videoEventsFor(int playerId) => _streams[playerId]!.stream;

  @override
  Future<void> play(int playerId) async {}

  @override
  Future<void> pause(int playerId) async {}

  @override
  Future<void> setLooping(int playerId, bool looping) async {}

  @override
  Future<void> setVolume(int playerId, double volume) async {}

  @override
  Future<void> seekTo(int playerId, Duration position) async {}

  @override
  Future<void> setPlaybackSpeed(int playerId, double speed) async {}

  @override
  Future<Duration> getPosition(int playerId) async => Duration.zero;

  @override
  Future<void> setMixWithOthers(bool mixWithOthers) async {}
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// A [VideoPlayerPlatform] that creates successfully (returns a real player ID
/// and emits a forceInitError event to trigger an init/play failure), but whose
/// [dispose()] throws a [PlatformException].
///
/// Used to verify that the viewer's `unawaited(dispose().catchError(...))` path
/// does NOT surface an uncaught async error when disposal fails after a
/// post-create load failure.
class _FailingDisposeVideoPlayerPlatform extends VideoPlayerPlatform {
  int disposeCallCount = 0;
  int nextPlayerId = 0;
  final Map<int, StreamController<VideoEvent>> _streams = {};

  /// Completed when dispose() is first entered.  Awaited inside
  /// [WidgetTester.runAsync] with a bounded real-zone timeout; timers created
  /// in that zone fire normally, unlike FakeAsync timers outside runAsync.
  final Completer<void> disposedCompleter = Completer<void>();

  @override
  Future<void> init() async {}

  @override
  Future<int?> createWithOptions(VideoCreationOptions options) async {
    return create(options.dataSource);
  }

  @override
  Future<int?> create(DataSource dataSource) async {
    final id = nextPlayerId++;
    final controller = StreamController<VideoEvent>(
      onListen: () {
        // Emit a PlatformException so initialize() throws, reaching the inner catch.
        _streams[id]!.addError(
          PlatformException(
            code: 'VideoError',
            message: 'Fake post-create init failure',
          ),
        );
      },
    );
    _streams[id] = controller;
    return id;
  }

  @override
  Future<void> dispose(int playerId) async {
    disposeCallCount++;
    if (!disposedCompleter.isCompleted) disposedCompleter.complete();
    // Throw to simulate a native disposal failure.
    // The production catch(.catchError) must absorb this without propagating
    // an uncaught async error while the viewer is showing its error UI.
    throw PlatformException(
      code: 'DisposalError',
      message: 'Fake native disposal failure',
    );
  }

  @override
  Widget buildView(int playerId) => const SizedBox.shrink();

  @override
  Stream<VideoEvent> videoEventsFor(int playerId) => _streams[playerId]!.stream;

  @override
  Future<void> play(int playerId) async {}

  @override
  Future<void> pause(int playerId) async {}

  @override
  Future<void> setLooping(int playerId, bool looping) async {}

  @override
  Future<void> setVolume(int playerId, double volume) async {}

  @override
  Future<void> seekTo(int playerId, Duration position) async {}

  @override
  Future<void> setPlaybackSpeed(int playerId, double speed) async {}

  @override
  Future<Duration> getPosition(int playerId) async => Duration.zero;

  @override
  Future<void> setMixWithOthers(bool mixWithOthers) async {}
}

/// Returns a [http.StreamedResponse] with the given [statusCode] whose body
/// stream is controlled by [bodyController].  The caller closes [bodyController]
/// to release a drain; leaving it open proves that the fix (listen+cancel)
/// completes without waiting for the stream to close.
http.StreamedResponse _streamedResponse(
  int statusCode,
  StreamController<List<int>> bodyController,
) => http.StreamedResponse(bodyController.stream, statusCode);

/// A no-op auth service (returns empty headers for any URL, including
/// non-relay URLs so the test media URL does not need a signed nsec).
MediaGetAuthService _noopAuth() =>
    MediaGetAuthService(baseUrl: 'https://relay.test', nsec: null);

/// A fake [http.Client] that calls [request.finalize()] and drains the
/// request body before returning a response.  If the request sink is not
/// closed, finalize() returns an open stream and the drain hangs — which is
/// exactly what the sink-fix prevents.
class _FinalizingFakeClient extends http.BaseClient {
  final http.StreamedResponse Function() responseBuilder;
  bool requestBodyDrained = false;

  _FinalizingFakeClient({required this.responseBuilder});

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    // Drain the finalized request body.  If sink.close() was not called this
    // stream never ends and the test times out — verifying the transport fix.
    await request.finalize().drain<void>();
    requestBodyDrained = true;
    return responseBuilder();
  }
}

/// A fake [http.Client] for viewer-path abort tests.
///
/// `send()` signals arrival immediately, then suspends until the request's
/// `abortTrigger` completes.  When the viewer's effect cleanup fires
/// `downloadRequestAbort.complete()`, that trigger arrives here and
/// `send()` throws [RequestAbortedException] — proving that unmounting
/// the widget closes the in-flight download through the actual viewer
/// abort-wiring path.
///
/// The fake requires an [http.AbortableStreamedRequest] with a non-null
/// trigger.  If the viewer's wiring is absent, `send()` throws [StateError],
/// which the viewer catches at its outer `catch (loadError)` boundary.  The
/// test then fails at the [abortObservedCompleter] deadline because the abort
/// is never observed — not immediately, but after the deadline expires.
///
/// No drain: this fake's only job is the abort-trigger chain.  Sink-close
/// correctness is covered separately by [_FinalizingFakeClient].
class _StallingAbortableClient extends http.BaseClient {
  final Completer<void> requestArrivedCompleter = Completer<void>();

  /// Completed when `abortObserved` is set; use with a deadline instead of sleeping.
  final Completer<void> abortObservedCompleter = Completer<void>();
  bool abortObserved = false;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    if (!requestArrivedCompleter.isCompleted) {
      requestArrivedCompleter.complete();
    }
    // Require an abortable request with a non-null trigger.  If the viewer's
    // AbortableStreamedRequest wiring or the abortTrigger: parameter is absent,
    // fail immediately — do NOT treat missing wiring as a successful abort.
    if (request case http.AbortableStreamedRequest(:final abortTrigger?)) {
      await abortTrigger;
      abortObserved = true;
      if (!abortObservedCompleter.isCompleted) {
        abortObservedCompleter.complete();
      }
      throw http.RequestAbortedException(request.url);
    }
    throw StateError(
      '_StallingAbortableClient: expected AbortableStreamedRequest with '
      'non-null abortTrigger; got ${request.runtimeType}. '
      'The viewer abort wiring is absent.',
    );
  }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    PathProviderPlatform.instance = _FakePathProviderPlatform();
  });

  // Transport: request sink must be closed before send().
  //
  // Red-with-old-code: before the `unawaited(request.sink.close())` fix,
  // _FinalizingFakeClient.send() drained an open stream and the test timed
  // out.  With the fix the drain completes immediately, the download succeeds,
  // and the video controller initializes.
  testWidgets(
    'Transport: request sink is closed before send() — fake drain probe',
    (tester) async {
      final fakePlayer = _FakeVideoPlayerPlatform();
      VideoPlayerPlatform.instance = fakePlayer;

      final fakeClient = _FinalizingFakeClient(
        responseBuilder: () =>
            http.StreamedResponse(Stream.value(<int>[0, 1, 2, 3]), 200),
      );
      addTearDown(fakeClient.close);

      await tester.runAsync(() async {
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(fakeClient),
            ],
            child: const MediaVideoViewerPage(
              videoUrl: 'https://relay.test/media/abc.mp4',
            ),
          ),
        );
        await Future<void>.delayed(const Duration(milliseconds: 300));
      });
      await tester.pump();

      expect(
        fakeClient.requestBodyDrained,
        isTrue,
        reason: 'request sink must be closed so send() can finalize the body',
      );
    },
  );

  // Transport: request sink must be closed before send().
  //
  // Real-IO loopback probes (TestWidgetsFlutterBinding intercepts HttpClient
  // within this suite) live in video_viewer_transport_test.dart, which uses
  // plain test() without TestWidgetsFlutterBinding.  The fake drain probe
  // below covers the same contract without the binding conflict.
  //
  //
  // Red-with-old-code: before the fix the `localController` was created but
  // only published to `controller.value` after a successful initialize()+play().
  // On error the outer `catch (loadError)` ran without ever calling
  // `localController.dispose()`.  With forceInitError=true the fake emits a
  // PlatformException; `initialize()` throws; the new inner catch calls
  // `dispose()` before rethrowing.  disposeCallCount >= 1 verifies it.
  testWidgets('F2r(a): VideoPlayerController is disposed when native init fails', (
    tester,
  ) async {
    final fakePlayer = _FakeVideoPlayerPlatform(forceInitError: true);
    VideoPlayerPlatform.instance = fakePlayer;

    // 200-ok response with a tiny immediate body so the download phase
    // completes and initializeVideo() reaches the VideoPlayerController.file()
    // path.  The _FinalizingFakeClient drains the request body, which proves
    // the sink is closed (if not, the drain hangs and the test times out).
    final fakeClient = _FinalizingFakeClient(
      responseBuilder: () =>
          http.StreamedResponse(Stream.value(<int>[0, 1, 2, 3]), 200),
    );
    addTearDown(fakeClient.close);

    await tester.runAsync(() async {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          disableAnimations: true,
          overrides: [
            mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
            mediaHttpClientProvider.overrideWithValue(fakeClient),
          ],
          child: const MediaVideoViewerPage(
            videoUrl: 'https://relay.test/media/abc.mp4',
          ),
        ),
      );
      // Give the initializeVideo() async chain time to complete: HTTP response,
      // file write, VideoPlayerController.initialize(), and dispose().
      await Future<void>.delayed(const Duration(milliseconds: 300));
    });
    await tester.pump();

    // The fake must have recorded at least one dispose() call, confirming
    // the native player was released even on an initialisation failure.
    expect(
      fakePlayer.disposeCallCount,
      greaterThanOrEqualTo(1),
      reason: 'VideoPlayerController must be disposed when initialize() throws',
    );
  });

  // F2r(b): close-during-error-body must cancel the stream and show the
  // error UI while the body is still open.
  //
  // Red-with-old-code: before the fix a 403 response body was consumed via
  // `response.stream.drain<void>()`, which suspends until the upstream closes
  // the stream.  With the fix, `_cancelVideoResponse(response)` subscribes and
  // immediately cancels.
  //
  // Discriminating assertion: (1) the body stream's onCancel fires while the
  // body is still open (never would with drain()), and (2) the error UI is
  // visible while the body remains open.  Restoring drain() breaks both.
  testWidgets('F2r(b): non-2xx error body is cancelled, not drained', (
    tester,
  ) async {
    final fakePlayer = _FakeVideoPlayerPlatform();
    VideoPlayerPlatform.instance = fakePlayer;

    // Body stream that NEVER closes — simulates a slow/stalled server.
    // drain() would block here indefinitely; _cancelVideoResponse completes
    // immediately by subscribing and cancelling.
    //
    // The Completer fires as soon as the stream's onCancel callback runs,
    // giving the test a bounded completion signal instead of a fixed sleep.
    // It is the cancellation of the body (not settling) that proves the fix:
    // the assertions below check that (a) the cancel fires while the body is
    // still open, and (b) the error UI is visible at that moment.
    final cancelledCompleter = Completer<void>();
    var bodyStreamCancelled = false;
    final stalledBody = StreamController<List<int>>(
      onCancel: () {
        bodyStreamCancelled = true;
        if (!cancelledCompleter.isCompleted) cancelledCompleter.complete();
      },
    );
    addTearDown(stalledBody.close);

    final fakeClient = _FinalizingFakeClient(
      responseBuilder: () => _streamedResponse(403, stalledBody),
    );
    addTearDown(fakeClient.close);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [
          mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
          mediaHttpClientProvider.overrideWithValue(fakeClient),
        ],
        child: const MediaVideoViewerPage(
          videoUrl: 'https://relay.test/media/abc.mp4',
        ),
      ),
    );

    // Wait for the body-cancellation signal rather than a fixed sleep.
    // disableAnimations: true stops BuzzLoadingIndicator from repeating so
    // pumpAndSettle converges once the error state is set.
    // With drain(), cancelledCompleter never completes and this times out.
    await tester.runAsync(
      () => cancelledCompleter.future.timeout(
        const Duration(seconds: 5),
        onTimeout: () => throw TimeoutException(
          'body-cancellation signal not received within 5 s',
        ),
      ),
    );
    await tester.pumpAndSettle();

    // (1) The body stream's onCancel must have fired — confirming listen+cancel
    //     was used, not drain().  With drain(), onCancel fires only when the
    //     whole drain completes (which never happens here).
    expect(
      bodyStreamCancelled,
      isTrue,
      reason:
          'error-body stream must be cancelled (listen+cancel), not drained',
    );

    // (2) The error UI must be visible while the body stream is still open
    //     (stalledBody was never closed).  _MediaLoadFailure shows this text
    //     when error.value is set — which only happens after _cancelVideoResponse
    //     completes and the HttpException propagates to the outer catch.
    //     With drain(), error.value is never set (drain hangs), so this
    //     assertion fails.
    expect(
      find.text('Failed to load video'),
      findsOneWidget,
      reason: 'error UI must be visible while the stalled body is still open',
    );

    // No controller is created before a 403 response.
    expect(
      fakePlayer.disposeCallCount,
      0,
      reason: 'No controller is created before a 403 response',
    );

    // (3) Explicitly unmount the viewer and verify no disposal fires from the
    //     teardown either — no controller was ever created.
    // Pass the same overrides so Riverpod's debug assertion
    // (_debugOverridesLength == overrides.length) does not fire.
    await tester.pumpWidget(
      WidgetHelpers.testable(
        disableAnimations: true,
        overrides: [
          mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
          mediaHttpClientProvider.overrideWithValue(fakeClient),
        ],
        child: const SizedBox.shrink(),
      ),
    );
    await tester.pumpAndSettle();
    expect(
      fakePlayer.disposeCallCount,
      0,
      reason: 'Unmounting after 403 must not dispose a non-existent controller',
    );
  });

  // F2r(c): VideoPlayerController created but never initialized must be
  // disposed when the widget is unmounted.
  //
  // Scenario: download succeeds, VideoPlayerController.file() is constructed
  // and registered in pendingController, but initialize() hangs forever (the
  // fake never emits an initialized event).  Unmounting fires the effect
  // cleanup which must dispose the pending controller via pendingController.
  //
  // Red-with-old-code: before the pendingController ref, localController lived
  // only in the async function's stack frame; the effect cleanup read only
  // controller.value (null until init completes) and disposed nothing — the
  // native player was leaked.
  testWidgets(
    'F2r(c): controller created but never initialized is disposed on unmount',
    (tester) async {
      final fakePlayer = _FakeVideoPlayerPlatform(neverInitialize: true);
      VideoPlayerPlatform.instance = fakePlayer;

      final fakeClient = _FinalizingFakeClient(
        responseBuilder: () =>
            http.StreamedResponse(Stream.value(<int>[0, 1, 2, 3]), 200),
      );
      addTearDown(fakeClient.close);

      await tester.runAsync(() async {
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(fakeClient),
            ],
            child: const MediaVideoViewerPage(
              videoUrl: 'https://relay.test/media/abc.mp4',
            ),
          ),
        );
        // Allow enough time for the download to complete and for
        // VideoPlayerController.file() to be constructed, but NOT long
        // enough for initialize() to complete (it never will).
        await Future<void>.delayed(const Duration(milliseconds: 300));

        // Now unmount — this triggers the effect cleanup with the pending
        // controller still in pendingController.value (never reached play()).
        // Pass the same overrides so Riverpod's debug assertion
        // (_debugOverridesLength == overrides.length) does not fire.
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(fakeClient),
            ],
            child: const SizedBox.shrink(),
          ),
        );
        await Future<void>.delayed(const Duration(milliseconds: 100));
      });
      await tester.pump();

      // The pending controller must have been disposed by the effect cleanup,
      // even though initialize() never returned.
      expect(
        fakePlayer.disposeCallCount,
        greaterThanOrEqualTo(1),
        reason:
            'pendingController must be disposed on unmount even if init never completes',
      );
    },
  );

  // F2r(d): createWithOptions() failure must show the error UI, not leave
  // the viewer in an infinite loading state.
  //
  // Scenario: the platform plugin's createWithOptions() itself throws a
  // PlatformException before returning a player ID.  video_player 2.11.1
  // creates _creatingCompleter at initialize():546 and completes it only on
  // the line AFTER await createWithOptions() (:587-590).  If creation throws,
  // _creatingCompleter is never completed, and dispose() awaits it at :682-683.
  // The old code `await localController.dispose()` in the catch block therefore
  // deadlocks: the outer catch never runs, error.value is never set, and the
  // viewer remains on the loading screen.
  //
  // Fix: `unawaited(localController.dispose().catchError(...))` in the catch
  // rethrows immediately so the outer catch sets error.value and shows the UI.
  //
  // Red-with-old-code: the old `await localController.dispose()` never returns
  // (dispose() waits on the uncompleted _creatingCompleter); the 300 ms window
  // ends with error.value still unset and the error UI absent — the assertion
  // fails.  With `unawaited(dispose())` the outer catch runs immediately and
  // sets error.value; the 300 ms window is enough for the fix to take effect.
  //
  // Note: create itself fails here, so _creatingCompleter is never completed
  // and the unawaited dispose() stalls at the same wait — this fix bypasses
  // the deadlock for the outer catch, but does not release the native player.
  // No dispose count is claimed for this case.
  testWidgets(
    'F2r(d): createWithOptions() failure shows error UI (not infinite spinner)',
    (tester) async {
      final fakePlayer = _FakeVideoPlayerPlatform(forceCreateError: true);
      VideoPlayerPlatform.instance = fakePlayer;

      final fakeClient = _FinalizingFakeClient(
        responseBuilder: () =>
            http.StreamedResponse(Stream.value(<int>[0, 1, 2, 3]), 200),
      );
      addTearDown(fakeClient.close);

      await tester.runAsync(() async {
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(fakeClient),
            ],
            child: const MediaVideoViewerPage(
              videoUrl: 'https://relay.test/media/abc.mp4',
            ),
          ),
        );
        // Allow the download to complete, createWithOptions() to throw, and
        // the outer catch to set error.value.  With the old `await dispose()`
        // the outer catch is blocked — `error.value` is never set and
        // `find.text('Failed to load video')` fails.
        await Future<void>.delayed(const Duration(milliseconds: 300));
      });
      await tester.pumpAndSettle();

      // The error UI must be visible: unawaited dispose + rethrow lets the
      // outer catch set error.value and show _MediaLoadFailure.
      // With the old `await dispose()` the viewer stalls and this fails.
      expect(
        find.text('Failed to load video'),
        findsOneWidget,
        reason:
            'createWithOptions() failure must show error UI, not infinite spinner',
      );
    },
  );

  // Detached disposal error handling: a successfully-created controller whose
  // init fails and whose dispose() ALSO throws must not surface an uncaught
  // async error.  The unawaited(dispose().catchError(...)) path in the inner
  // catch must absorb the disposal exception with logging.
  //
  // Red-with-old-code (before the .catchError addition): the detached future
  // throws PlatformException unhandled; the Flutter test binding's own
  // FlutterError.onError captures it and fails the test.  With .catchError
  // the error is absorbed before reaching the binding's handler.
  //
  // Note: no FlutterError.onError override is needed here.  testWidgets
  // automatically fails the test if any uncaught Flutter error reaches the
  // binding's handler — the test passing IS the assertion that no uncaught
  // error occurred.
  testWidgets(
    'F2r(d)+: post-create disposal failure shows error UI and no uncaught error',
    (tester) async {
      final fakeClient = _FinalizingFakeClient(
        responseBuilder: () =>
            http.StreamedResponse(Stream.value(<int>[0, 1, 2, 3]), 200),
      );
      addTearDown(fakeClient.close);

      // Construct the fake and its completer inside runAsync so that the
      // bounded await below is registered in the real scheduler zone, not
      // FakeAsync.  A Duration-based timeout or future created outside runAsync
      // becomes a FakeAsync timer; the binding never auto-advances fake time in
      // testWidgets, so it hangs to the 30 s outer runner timeout instead of
      // failing promptly.  Inside runAsync, timers are dispatched to the real
      // event loop and fire normally.
      //
      // Zone ownership of the completion: the viewer mounts here, so
      // initializeVideo() starts in the real zone.  The inner catch calls
      // unawaited(localController.dispose().catchError(...)) also in the real
      // zone.  _FailingDisposeVideoPlayerPlatform.dispose() runs synchronously
      // inside that detached future, completing disposedCompleter before any
      // suspension.  The await below (also in the real zone) therefore resolves
      // as soon as the microtask queue drains the detached disposal future.
      //
      // Removal check: removing only the unawaited disposal call leaves
      // disposedCompleter never completed; the 5 s timeout fires, failing the
      // test immediately and deterministically.
      await tester.runAsync(() async {
        final fakePlayer = _FailingDisposeVideoPlayerPlatform();
        VideoPlayerPlatform.instance = fakePlayer;

        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(fakeClient),
            ],
            child: const MediaVideoViewerPage(
              videoUrl: 'https://relay.test/media/abc.mp4',
            ),
          ),
        );

        // Wait for the disposal path to be entered, bounded by a real-zone
        // timeout.  The viewer downloads the body, creates the player, emits a
        // PlatformException from the event stream, enters the inner catch, and
        // calls unawaited(dispose().catchError(...)).  dispose() completes
        // disposedCompleter synchronously at its entry point before throwing.
        await fakePlayer.disposedCompleter.future.timeout(
          const Duration(seconds: 5),
          onTimeout: () => throw TimeoutException(
            'dispose() was not entered within 5 s '
            '— the unawaited disposal path may have been removed',
          ),
        );

        // Pump to flush the error state set in the outer catch after the
        // detached disposal starts.
        await tester.pumpAndSettle();

        // Error UI must appear — disposal failure must not block the outer catch.
        expect(
          find.text('Failed to load video'),
          findsOneWidget,
          reason: 'post-create disposal failure must still show error UI',
        );
        // dispose() must have been called — confirms the unawaited disposal path ran.
        expect(
          fakePlayer.disposeCallCount,
          greaterThanOrEqualTo(1),
          reason: 'dispose() must have been called on the failing player',
        );
        // The test passing without a framework error IS the assertion that
        // the disposal PlatformException was absorbed by .catchError and did
        // not reach the binding's uncaught-error handler.
      });
    },
  );

  // Viewer-path abort: unmounting the widget while the download is in-flight
  // must abort the HTTP request through the viewer's own wiring.
  //
  // The viewer creates an AbortableStreamedRequest with abortTrigger wired to
  // downloadRequestAbort.value (a Completer).  The effect cleanup calls
  // activeRequestAbort.complete() on unmount, which fires abortTrigger.
  //
  // Red-with-reverted-wiring: removing only `activeRequestAbort.complete()`
  // from the cleanup leaves the trigger pending and abortObserved stays false
  // at the deadline — the abortObservedCompleter times out.
  // Deleting the whole AbortableStreamedRequest / abortTrigger wiring causes
  // the fake to throw StateError from send(); the viewer catches it as a load
  // error, so abortObserved is never set and the test fails at the deadline.
  testWidgets(
    'Viewer abort-path: unmount fires abortTrigger and cancels in-flight download',
    (tester) async {
      final fakePlayer = _FakeVideoPlayerPlatform();
      VideoPlayerPlatform.instance = fakePlayer;

      final stallingClient = _StallingAbortableClient();
      addTearDown(stallingClient.close);

      await tester.runAsync(() async {
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(stallingClient),
            ],
            child: const MediaVideoViewerPage(
              videoUrl: 'https://relay.test/media/abort-test.mp4',
            ),
          ),
        );

        // Wait for the viewer to start the download and reach the stall point.
        await stallingClient.requestArrivedCompleter.future.timeout(
          const Duration(seconds: 5),
          onTimeout: () => throw TimeoutException(
            'Viewer did not start download within 5 s',
          ),
        );

        // Verify abort has NOT fired yet (before unmount).
        expect(
          stallingClient.abortObserved,
          isFalse,
          reason: 'abort must not fire before unmount',
        );

        // Now unmount — this fires the effect cleanup which calls
        // activeRequestAbort.complete(), completing abortTrigger.
        // Pass the same overrides so Riverpod's debug assertion does not fire.
        await tester.pumpWidget(
          WidgetHelpers.testable(
            disableAnimations: true,
            overrides: [
              mediaGetAuthServiceProvider.overrideWithValue(_noopAuth()),
              mediaHttpClientProvider.overrideWithValue(stallingClient),
            ],
            child: const SizedBox.shrink(),
          ),
        );
        // Pump once to flush the effect cleanup microtasks that fire during
        // the widget-tree disposal (useEffect cleanup in Flutter hooks runs
        // synchronously in the pumpWidget call above, but the Completer
        // completion and the abortTrigger await chain resolve on subsequent
        // microtask turns — yield here so they settle before the deadline).
        await tester.pump();

        // Wait for the abort to be observed with a deadline — no sleep.
        // Deleting activeRequestAbort.complete() in cleanup → times out here.
        await stallingClient.abortObservedCompleter.future.timeout(
          const Duration(seconds: 5),
          onTimeout: () => throw TimeoutException(
            'Abort was not observed within 5 s after unmount',
          ),
        );
      });
      await tester.pump();

      // The fake must have received the abort — proving that the viewer's
      // effect cleanup correctly wired the Completer to the AbortableStreamedRequest.
      expect(
        stallingClient.abortObserved,
        isTrue,
        reason:
            'viewer unmount must complete abortTrigger and cancel the in-flight download',
      );
    },
  );
}
