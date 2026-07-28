part of '../compose_bar.dart';

const _nativeAttachmentPopoverChannel = MethodChannel(
  'buzz/native_attachment_popover',
);

class _IOSAttachmentPopoverController {
  bool _isPresenting = false;

  Future<bool> present({
    required BuildContext sourceContext,
    required Future<void> Function(XFile image) onCapture,
    required Future<void> Function(List<XFile> photos) onChoosePhotos,
    required VoidCallback onAllPhotos,
    required VoidCallback onVideo,
    required VoidCallback onFiles,
  }) async {
    if (defaultTargetPlatform != TargetPlatform.iOS || _isPresenting) {
      return _isPresenting;
    }

    final supported =
        await _nativeAttachmentPopoverChannel.invokeMethod<bool>(
          'isSupported',
        ) ??
        false;
    if (!supported || !sourceContext.mounted) return false;

    final renderObject = sourceContext.findRenderObject();
    if (renderObject is! RenderBox || !renderObject.hasSize) return false;
    final origin = renderObject.localToGlobal(Offset.zero);

    _isPresenting = true;
    _nativeAttachmentPopoverChannel.setMethodCallHandler((call) async {
      switch (call.method) {
        case 'cameraCaptured':
          if (call.arguments case final String path) {
            await processCapturedImage(XFile(path), onCapture);
          }
        case 'photosSelected':
          final paths = (call.arguments as List<Object?>? ?? const [])
              .whereType<String>()
              .toList();
          if (paths.isNotEmpty) {
            await processTemporaryImages([
              for (final path in paths) XFile(path),
            ], onChoosePhotos);
          }
        case 'pickAllPhotos':
          onAllPhotos();
        case 'pickVideo':
          onVideo();
        case 'pickFiles':
          onFiles();
        case 'dismissed':
          _isPresenting = false;
      }
    });

    try {
      final didPresent =
          await _nativeAttachmentPopoverChannel.invokeMethod<bool>('present', {
            'x': origin.dx,
            'y': origin.dy,
            'width': renderObject.size.width,
            'height': renderObject.size.height,
          }) ??
          false;
      if (!didPresent) {
        _isPresenting = false;
        _nativeAttachmentPopoverChannel.setMethodCallHandler(null);
      }
      return didPresent;
    } on PlatformException {
      _isPresenting = false;
      _nativeAttachmentPopoverChannel.setMethodCallHandler(null);
      return false;
    }
  }

  Future<void> dispose() async {
    if (_isPresenting) {
      await _nativeAttachmentPopoverChannel.invokeMethod<void>('dismiss');
    }
    _isPresenting = false;
    _nativeAttachmentPopoverChannel.setMethodCallHandler(null);
  }
}
