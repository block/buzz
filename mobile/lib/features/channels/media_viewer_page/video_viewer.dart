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

        // ExoPlayer supports the request headers on every range request, so
        // keep Android on its streaming path. iOS uses the authenticated local
        // copy below because AVPlayer can drop those headers after the first
        // request.
        if (Platform.isAndroid) {
          VideoPlayerController? streamingController;
          try {
            streamingController = VideoPlayerController.networkUrl(
              uri,
              httpHeaders: auth.headersFor(videoUrl),
            );
            await streamingController.initialize();
            await streamingController.play();
            if (disposed) {
              await streamingController.dispose();
              return;
            }
            controller.value = streamingController;
            return;
          } catch (_) {
            if (streamingController != null) {
              await streamingController.dispose();
            }
            // Fall through to the authenticated local-file path only when the
            // streaming controller cannot initialize.
          }
        }

        try {
          final client = ref.read(mediaHttpClientProvider);
          final requestAbort = Completer<void>();
          downloadRequestAbort.value = requestAbort;
          final request = http.AbortableStreamedRequest(
            'GET',
            uri,
            abortTrigger: requestAbort.future,
          )..headers.addAll(auth.headersFor(videoUrl));
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
            await response.stream.drain<void>();
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
          await localController.initialize();
          await localController.play();
          if (disposed) {
            await localController.dispose();
            await deleteVideoFile();
            return;
          }
          controller.value = localController;
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
                child: _MediaViewerCircleButton(
                  key: const ValueKey('message-media-video-viewer-close'),
                  icon: LucideIcons.x,
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
