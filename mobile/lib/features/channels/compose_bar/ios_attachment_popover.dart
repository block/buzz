part of '../compose_bar.dart';

const _nativeAttachmentPopoverChannel = MethodChannel(
  'buzz/native_attachment_popover',
);

final _iosAttachmentPopoverCoordinator = _IOSAttachmentPopoverCoordinator(
  _nativeAttachmentPopoverChannel,
);

class _IOSAttachmentPopoverCallbacks {
  final Future<void> Function(XFile image) onCapture;
  final Future<void> Function(List<XFile> photos) onChoosePhotos;
  final VoidCallback onAllPhotos;
  final VoidCallback onVideo;
  final VoidCallback onFiles;

  const _IOSAttachmentPopoverCallbacks({
    required this.onCapture,
    required this.onChoosePhotos,
    required this.onAllPhotos,
    required this.onVideo,
    required this.onFiles,
  });
}

class _IOSAttachmentPopoverCoordinator {
  static final Object _cancelledSupportCheck = Object();

  final MethodChannel _channel;

  Object? _activeOwner;
  _IOSAttachmentPopoverCallbacks? _callbacks;
  Completer<void>? _pendingSupportCancellation;
  bool _didPresent = false;
  bool _handlerInstalled = false;

  _IOSAttachmentPopoverCoordinator(this._channel);

  Future<bool> present({
    required Object owner,
    required BuildContext sourceContext,
    required Future<void> Function(XFile image) onCapture,
    required Future<void> Function(List<XFile> photos) onChoosePhotos,
    required VoidCallback onAllPhotos,
    required VoidCallback onVideo,
    required VoidCallback onFiles,
  }) async {
    if (defaultTargetPlatform != TargetPlatform.iOS) return false;
    if (_activeOwner case final activeOwner?) {
      if (identical(activeOwner, owner)) return true;
      if (!_didPresent) _clearOwner(activeOwner);
      return _didPresent;
    }

    final supportCancellation = Completer<void>();
    _activeOwner = owner;
    _pendingSupportCancellation = supportCancellation;
    _callbacks = _IOSAttachmentPopoverCallbacks(
      onCapture: onCapture,
      onChoosePhotos: onChoosePhotos,
      onAllPhotos: onAllPhotos,
      onVideo: onVideo,
      onFiles: onFiles,
    );
    _ensureHandler();

    try {
      final supportResult = await Future.any<Object?>([
        _channel.invokeMethod<bool>('isSupported'),
        supportCancellation.future.then<Object?>((_) => _cancelledSupportCheck),
      ]);
      if (identical(supportResult, _cancelledSupportCheck)) return true;
      if (identical(_pendingSupportCancellation, supportCancellation)) {
        _pendingSupportCancellation = null;
      }
      if (!identical(_activeOwner, owner)) return false;
      final supported = supportResult == true;
      if (!supported || !sourceContext.mounted) {
        _clearOwner(owner);
        return false;
      }

      final renderObject = sourceContext.findRenderObject();
      if (renderObject is! RenderBox || !renderObject.hasSize) {
        _clearOwner(owner);
        return false;
      }
      final origin = renderObject.localToGlobal(Offset.zero);

      _didPresent = true;
      final didPresent =
          await _channel.invokeMethod<bool>('present', {
            'x': origin.dx,
            'y': origin.dy,
            'width': renderObject.size.width,
            'height': renderObject.size.height,
          }) ??
          false;
      if (!identical(_activeOwner, owner)) return false;
      if (!didPresent) _clearOwner(owner);
      return didPresent;
    } on PlatformException {
      _clearOwner(owner);
      return false;
    }
  }

  Future<void> disposeOwner(Object owner) async {
    if (!identical(_activeOwner, owner)) return;

    _callbacks = null;
    if (!_didPresent) {
      _clearOwner(owner);
      return;
    }

    try {
      await _channel.invokeMethod<void>('dismiss');
    } on PlatformException {
      // The native bridge is unavailable, so there is nothing left to dismiss.
    } on MissingPluginException {
      // The native bridge is unavailable, so there is nothing left to dismiss.
    } finally {
      _clearOwner(owner);
    }
  }

  void _ensureHandler() {
    if (_handlerInstalled) return;
    _handlerInstalled = true;
    _channel.setMethodCallHandler(_handleMethodCall);
  }

  Future<void> _handleMethodCall(MethodCall call) async {
    final callbacks = _callbacks;
    switch (call.method) {
      case 'cameraCaptured':
        if (call.arguments case final String path) {
          final activeCallbacks = callbacks;
          if (activeCallbacks != null) {
            await processCapturedImage(XFile(path), activeCallbacks.onCapture);
          }
        }
      case 'photosSelected':
        final paths = (call.arguments as List<Object?>? ?? const [])
            .whereType<String>()
            .toList();
        if (callbacks != null && paths.isNotEmpty) {
          await processTemporaryImages([
            for (final path in paths) XFile(path),
          ], callbacks.onChoosePhotos);
        }
      case 'pickAllPhotos':
        callbacks?.onAllPhotos();
      case 'pickVideo':
        callbacks?.onVideo();
      case 'pickFiles':
        callbacks?.onFiles();
      case 'dismissed':
        _activeOwner = null;
        _callbacks = null;
        _didPresent = false;
    }
  }

  void _clearOwner(Object owner) {
    if (!identical(_activeOwner, owner)) return;
    final supportCancellation = _pendingSupportCancellation;
    _pendingSupportCancellation = null;
    if (supportCancellation != null && !supportCancellation.isCompleted) {
      supportCancellation.complete();
    }
    _activeOwner = null;
    _callbacks = null;
    _didPresent = false;
  }
}

class _IOSAttachmentPopoverController {
  Future<bool> present({
    required BuildContext sourceContext,
    required Future<void> Function(XFile image) onCapture,
    required Future<void> Function(List<XFile> photos) onChoosePhotos,
    required VoidCallback onAllPhotos,
    required VoidCallback onVideo,
    required VoidCallback onFiles,
  }) => _iosAttachmentPopoverCoordinator.present(
    owner: this,
    sourceContext: sourceContext,
    onCapture: onCapture,
    onChoosePhotos: onChoosePhotos,
    onAllPhotos: onAllPhotos,
    onVideo: onVideo,
    onFiles: onFiles,
  );

  Future<void> dispose() => _iosAttachmentPopoverCoordinator.disposeOwner(this);
}
