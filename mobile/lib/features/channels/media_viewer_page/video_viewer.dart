part of '../media_viewer_page.dart';

class MediaVideoViewerPage extends HookConsumerWidget {
  final String videoUrl;
  final String? posterUrl;
  final VoidCallback? onReply;

  static const _dismissThreshold = 100.0;
  static const _dismissVelocity = 700.0;
  static const _backgroundFadeDivisor = 300.0;

  const MediaVideoViewerPage({
    super.key,
    required this.videoUrl,
    this.posterUrl,
    this.onReply,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = useState<VideoPlayerController?>(null);
    // Tracks a VideoPlayerController that has been created (i.e. platform
    // resources allocated via createWithOptions()) but whose initialize()+play()
    // chain has not yet completed or failed.  The effect cleanup path disposes
    // this directly so a close-during-init does not leak the native player,
    // even when the async chain is suspended waiting for the initialized event
    // or for play() to return.
    final pendingController = useRef<VideoPlayerController?>(null);
    final videoFile = useRef<File?>(null);
    final downloadRequestAbort = useRef<Completer<void>?>(null);
    final downloadSubscription = useRef<StreamSubscription<List<int>>?>(null);
    final downloadSink = useRef<IOSink?>(null);
    final initializeFuture = useState<Future<void>?>(null);
    final error = useState<String?>(null);
    final dragOffset = useState(0.0);
    final isDragging = useState(false);
    final snapBackController = useAnimationController(
      duration: const Duration(milliseconds: 200),
    );

    Future<void> deleteVideoFile() async {
      final file = videoFile.value;
      videoFile.value = null;
      if (file == null) return;
      try {
        if (await file.exists()) await file.delete();
      } on FileSystemException {
        // Temporary storage cleanup should not make closing the viewer fail.
      }
    }

    useEffect(() {
      var disposed = false;
      Future<void> initializeVideo() async {
        final auth = ref.read(mediaGetAuthServiceProvider);
        final uri = Uri.parse(videoUrl);

        // All platforms: download to an authenticated local file so the proof
        // is bound at request time rather than frozen into controller headers.
        // (iOS already used this path; Android previously used streaming headers
        // but video_player_android 2.9.5 freezes those headers into static
        // DefaultHttpDataSource request properties — a proof minted at
        // controller creation time becomes stale after 60 s, causing seeks
        // outside the buffer to fail with expiry rejection.)
        try {
          final client = ref.read(mediaHttpClientProvider);
          final requestAbort = Completer<void>();
          downloadRequestAbort.value = requestAbort;
          final request = http.AbortableStreamedRequest(
            'GET',
            uri,
            abortTrigger: requestAbort.future,
          )..headers.addAll(auth.headersFor(videoUrl));
          // A GET carries no request body.  StreamedRequest's sink MUST be
          // closed to signal end-of-stream: IOClient.send() awaits
          // stream.pipe(ioRequest) before returning a response, and pipe
          // blocks until the source stream ends.  Without close(), every
          // download hangs in loading until the request is aborted.
          // close() is unawaited because it may not complete until after
          // the pipe is in progress (streamed_request.dart:15-29).
          unawaited(request.sink.close());
          late final http.StreamedResponse response;
          try {
            response = await client.send(request);
          } finally {
            if (downloadRequestAbort.value == requestAbort) {
              downloadRequestAbort.value = null;
            }
          }
          if (disposed) {
            await _cancelVideoResponse(response);
            return;
          }
          if (response.statusCode < 200 || response.statusCode >= 300) {
            // Cancel (not drain) the error-body stream so a stalled server body
            // cannot hold the download open.  `drain()` waits for the upstream
            // to close the stream; `_cancelVideoResponse` subscribes and
            // immediately cancels, which closes the underlying connection without
            // waiting for the full response body [F2r(b)].
            await _cancelVideoResponse(response);
            throw HttpException(
              'Video download failed (${response.statusCode})',
              uri: uri,
            );
          }

          final responseSubscription = response.stream.listen(null)..pause();
          downloadSubscription.value = responseSubscription;
          final directory = await getTemporaryDirectory();
          if (disposed) {
            await responseSubscription.cancel();
            if (downloadSubscription.value == responseSubscription) {
              downloadSubscription.value = null;
            }
            return;
          }
          final file = File(
            '${directory.path}${Platform.pathSeparator}'
            'buzz-video-${DateTime.now().microsecondsSinceEpoch}'
            '${_videoFileExtension(uri)}',
          );
          videoFile.value = file;
          final sink = file.openWrite();
          downloadSink.value = sink;
          final completed = Completer<void>();
          responseSubscription
            ..onData(sink.add)
            ..onError((Object error, StackTrace stackTrace) async {
              await sink.close();
              if (!completed.isCompleted) {
                completed.completeError(error, stackTrace);
              }
            })
            ..onDone(() async {
              await sink.close();
              if (!completed.isCompleted) completed.complete();
            })
            ..resume();
          await completed.future;
          downloadSubscription.value = null;
          downloadSink.value = null;
          if (disposed) {
            await deleteVideoFile();
            return;
          }

          final localController = VideoPlayerController.file(file);
          // Register as pending BEFORE the first async suspension
          // (initialize()) so the effect cleanup can always reach it.
          // video_player 2.11.1 allocates the native player synchronously
          // inside createWithOptions() before _creatingCompleter completes;
          // a close arriving at any point after this line will find the
          // controller in pendingController and dispose it correctly.
          pendingController.value = localController;
          // Own the controller before any async suspension so a failed
          // initialize() or play() — or a disposal that races with init —
          // can always call dispose() unconditionally [F2r(a)].
          // video_player 2.11.1 completes the init future with an error on
          // native failure but does NOT dispose the player; Android 2.9.5
          // retains the native player until explicit disposal.  Without this
          // wrapper, a PlatformException from initialize() unwinds to the
          // outer catch where controller.value is still null, so the cleanup
          // teardown's `if (activeController != null)` guard silently skips
          // disposal — leaking the native player and its event subscription.
          try {
            await localController.initialize();
            if (disposed) {
              // Effect cleanup will also see pendingController.value and
              // dispose it; clear the ref here to avoid a double-dispose.
              pendingController.value = null;
              await localController.dispose();
              await deleteVideoFile();
              return;
            }
            await localController.play();
            if (disposed) {
              pendingController.value = null;
              await localController.dispose();
              await deleteVideoFile();
              return;
            }
            pendingController.value = null;
            controller.value = localController;
          } catch (_) {
            // Start disposal without awaiting it, then rethrow immediately.
            //
            // video_player 2.11.1 initialize() creates _creatingCompleter at
            // the top of the method, then awaits createWithOptions() before
            // completing it (video_player.dart:546,587-590).  If
            // createWithOptions() itself throws, _creatingCompleter is never
            // completed, and dispose() waits on it unconditionally at :682-683.
            // Awaiting dispose() here would therefore deadlock: the outer catch
            // never sets error.value, the error UI is never shown, and the
            // viewer is left in an infinite loading state.
            //
            // Note: if createWithOptions() throws, _creatingCompleter is never
            // completed, so the unawaited disposal stalls at the same wait.
            // This bypasses the deadlock for the outer catch but does not
            // release the native player in the create-failure case.  After a
            // successful create, _creatingCompleter is completed at :590, so
            // the detached disposal runs normally; errors from that detached
            // future are caught and logged below rather than becoming uncaught
            // async errors [F2r(d)].
            pendingController.value = null;
            unawaited(
              localController.dispose().catchError((Object disposeError) {
                debugPrint(
                  '[VideoViewer] dispose() failed after load error: $disposeError',
                );
              }),
            );
            rethrow;
          }
        } catch (loadError) {
          if (!disposed) error.value = loadError.toString();
        }
      }

      initializeFuture.value = initializeVideo();
      return () {
        disposed = true;
        final activeRequestAbort = downloadRequestAbort.value;
        if (activeRequestAbort != null && !activeRequestAbort.isCompleted) {
          activeRequestAbort.complete();
        }
        unawaited(downloadSubscription.value?.cancel() ?? Future.value());
        unawaited(downloadSink.value?.close() ?? Future.value());
        // Dispose whichever controller is reachable: a controller that has
        // finished init+play and been published to controller.value, OR one
        // that is still mid-init (registered in pendingController before the
        // first await).  Exactly one of these is non-null at any moment;
        // clearing both refs prevents a double-dispose if initializeVideo()
        // races with the teardown.
        final activePending = pendingController.value;
        pendingController.value = null;
        if (activePending != null) unawaited(activePending.dispose());
        final activeController = controller.value;
        if (activeController != null) unawaited(activeController.dispose());
        unawaited(deleteVideoFile());
      };
    }, [videoUrl]);

    void animateSnapBack() {
      isDragging.value = false;
      if (MediaQuery.disableAnimationsOf(context)) {
        dragOffset.value = 0;
        return;
      }
      final tween = Tween<double>(begin: dragOffset.value, end: 0);
      void listener() => dragOffset.value = tween.evaluate(snapBackController);
      snapBackController
        ..stop()
        ..reset()
        ..addListener(listener);
      snapBackController
          .animateWith(
            SpringSimulation(
              SpringDescription.withDurationAndBounce(
                duration: const Duration(milliseconds: 260),
                bounce: 0.14,
              ),
              0,
              1,
              0,
              snapToEnd: true,
            ),
          )
          .whenCompleteOrCancel(
            () => snapBackController.removeListener(listener),
          );
    }

    Future<void> replyInThread() async {
      final callback = onReply;
      if (callback == null) return;
      final route = ModalRoute.of(context);
      controller.value?.pause();
      await Navigator.of(context).maybePop();
      await route?.completed;
      callback();
    }

    final viewportHeight = MediaQuery.sizeOf(context).height;
    final dragProgress = (dragOffset.value / viewportHeight).clamp(0.0, 1.0);
    final videoScale = 1 - (dragProgress * 0.1);
    final chromeOpacity = (1 - (dragOffset.value / 160)).clamp(0.0, 1.0);
    return Scaffold(
      key: const ValueKey('message-media-video-viewer'),
      backgroundColor: Colors.black.withValues(
        alpha: (1 - (dragOffset.value / _backgroundFadeDivisor)).clamp(
          0.3,
          1.0,
        ),
      ),
      body: Stack(
        children: [
          Positioned.fill(
            child: Transform.translate(
              offset: Offset(0, dragOffset.value),
              child: Transform.scale(
                scale: videoScale,
                child: GestureDetector(
                  key: const ValueKey('message-media-video-viewer-gesture'),
                  behavior: HitTestBehavior.opaque,
                  onVerticalDragStart: (_) {
                    snapBackController.stop();
                    isDragging.value = true;
                  },
                  onVerticalDragUpdate: (details) {
                    if (!isDragging.value) return;
                    dragOffset.value = (dragOffset.value + details.delta.dy)
                        .clamp(0.0, viewportHeight)
                        .toDouble();
                  },
                  onVerticalDragEnd: (details) {
                    isDragging.value = false;
                    final velocity = details.primaryVelocity ?? 0;
                    if (dragOffset.value > _dismissThreshold ||
                        velocity > _dismissVelocity) {
                      controller.value?.pause();
                      Navigator.of(context).maybePop();
                      return;
                    }
                    animateSnapBack();
                  },
                  onVerticalDragCancel: animateSnapBack,
                  child: SafeArea(
                    child: Center(
                      child: FutureBuilder<void>(
                        future: initializeFuture.value,
                        builder: (context, snapshot) {
                          if (error.value != null || snapshot.hasError) {
                            return const _MediaLoadFailure(
                              message: 'Failed to load video',
                              icon: LucideIcons.videoOff,
                            );
                          }

                          final videoController = controller.value;
                          if (videoController == null ||
                              !videoController.value.isInitialized) {
                            return _VideoLoadingPoster(posterUrl: posterUrl);
                          }

                          return AspectRatio(
                            aspectRatio: videoController.value.aspectRatio,
                            child: VideoPlayer(videoController),
                          );
                        },
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          PositionedDirectional(
            top: Grid.sm,
            end: Grid.sm,
            child: Opacity(
              opacity: chromeOpacity,
              child: SafeArea(
                child: _MediaViewerCloseButton(
                  key: const ValueKey('message-media-video-viewer-close'),
                  tooltip: 'Close video viewer',
                  onPressed: () => Navigator.of(context).maybePop(),
                ),
              ),
            ),
          ),
          PositionedDirectional(
            bottom: 0,
            start: 0,
            end: 0,
            child: Opacity(
              opacity: chromeOpacity,
              child: SafeArea(
                child: _VideoViewerBottomControls(
                  controller: controller.value,
                  onReply: onReply == null
                      ? null
                      : () => unawaited(replyInThread()),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

Future<void> _cancelVideoResponse(http.StreamedResponse response) {
  return response.stream.listen((_) {}).cancel();
}

String _videoFileExtension(Uri uri) {
  final path = uri.path;
  final extensionStart = path.lastIndexOf('.');
  if (extensionStart < 0 || extensionStart == path.length - 1) return '.mp4';
  final extension = path.substring(extensionStart);
  return RegExp(r'^\.[A-Za-z0-9]{1,10}$').hasMatch(extension)
      ? extension
      : '.mp4';
}

class _VideoLoadingPoster extends StatelessWidget {
  final String? posterUrl;

  const _VideoLoadingPoster({required this.posterUrl});

  @override
  Widget build(BuildContext context) {
    return AspectRatio(
      aspectRatio: 16 / 9,
      child: Stack(
        fit: StackFit.expand,
        children: [
          if (posterUrl != null)
            MediaImage(
              url: posterUrl!,
              fit: BoxFit.cover,
              errorBuilder: (_, _, _) => _videoPlaceholder(context),
            )
          else
            _videoPlaceholder(context),
          const ColoredBox(color: Color.fromRGBO(0, 0, 0, 0.24)),
          const Center(
            child: BuzzLoadingIndicator(
              size: 44,
              color: Colors.white,
              semanticLabel: 'Loading video',
            ),
          ),
        ],
      ),
    );
  }

  Widget _videoPlaceholder(BuildContext context) {
    return ColoredBox(
      color: context.colors.surfaceContainerHighest,
      child: Icon(
        LucideIcons.video,
        size: 40,
        color: context.colors.onSurfaceVariant,
      ),
    );
  }
}
