import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// UIKit's horizontally scrolling photo strip, using the app's image providers.
/// Authentication and image-cache isolation remain owned by Flutter.
class IosMediaCarousel extends HookWidget {
  const IosMediaCarousel({
    super.key,
    required this.providers,
    required this.labels,
    required this.width,
    required this.gutter,
    required this.gap,
    required this.trailingPadding,
    required this.radius,
    required this.onOpen,
    required this.onScroll,
    required this.onPageChanged,
  });

  static const viewType = 'buzz/ios_media_carousel';
  final List<ImageProvider<Object>> providers;
  final List<String> labels;
  final double width;
  final double gutter;
  final double gap;
  final double trailingPadding;
  final double radius;
  final ValueChanged<int> onOpen;
  final ValueChanged<double> onScroll;
  final ValueChanged<int> onPageChanged;

  @override
  Widget build(BuildContext context) {
    final channel = useState<MethodChannel?>(null);
    final latest = useRef(this)..value = this;
    final brightness = Theme.of(context).brightness.name;
    final labelSignature = Object.hashAll(labels);
    final rtl = Directionality.of(context) == TextDirection.rtl;
    final configuration = <String, Object>{
      'labels': labels,
      'width': width,
      'gutter': gutter,
      'gap': gap,
      'trailingPadding': trailingPadding,
      'radius': radius,
      'brightness': brightness,
      'rtl': rtl,
    };
    useEffect(() {
      final bridge = channel.value;
      if (bridge == null) return null;
      final images = _CarouselImages(bridge);
      channel.value!.setMethodCallHandler((call) async {
        final widget = latest.value;
        final argument = call.arguments;
        if (call.method == 'cancelImage' && argument is String) {
          images.cancel(argument);
        } else if (call.method == 'subscribeImage' && argument is Map) {
          final id = argument['id'];
          final index = argument['index'];
          if (id is String &&
              index is int &&
              index >= 0 &&
              index < widget.providers.length) {
            images.subscribe(
              id,
              widget.providers[index],
              createLocalImageConfiguration(context),
            );
          }
        } else if (call.method == 'scroll' && argument is num) {
          widget.onScroll(argument.toDouble());
        } else if (argument is int &&
            argument >= 0 &&
            argument < widget.providers.length) {
          if (call.method == 'open') widget.onOpen(argument);
          if (call.method == 'page') widget.onPageChanged(argument);
        }
        return null;
      });
      // UIKit requests visible thumbnails only after the Dart handler exists.
      unawaited(bridge.invokeMethod<void>('configure', configuration));
      return () {
        images.dispose();
        bridge.setMethodCallHandler(null);
      };
    }, [channel.value]);
    useEffect(
      () {
        final bridge = channel.value;
        if (bridge != null) {
          unawaited(bridge.invokeMethod<void>('configure', configuration));
        }
        return null;
      },
      [
        width,
        gutter,
        gap,
        trailingPadding,
        radius,
        brightness,
        rtl,
        labelSignature,
      ],
    );

    return UiKitView(
      viewType: viewType,
      creationParams: configuration,
      creationParamsCodec: const StandardMessageCodec(),
      gestureRecognizers: {
        Factory<HorizontalDragGestureRecognizer>(
          HorizontalDragGestureRecognizer.new,
        ),
      },
      onPlatformViewCreated: (id) {
        channel.value = MethodChannel('$viewType/$id');
      },
    );
  }
}

// Keep native subscriptions and unresolved provider loads bounded together.
// A cancelled load keeps its slot until its first frame/error. MediaImageProvider
// aborts stalled HTTP requests at its deadline, allowing queued cells to progress
// without releasing slots while their transports are still running.
class _CarouselImages {
  _CarouselImages(this.channel);
  final MethodChannel channel;
  final _queued = <String, _CarouselImage>{};
  final _running = <String, _CarouselImage>{};
  bool _disposed = false;
  bool _draining = false;

  void subscribe(
    String id,
    ImageProvider<Object> provider,
    ImageConfiguration config,
  ) {
    if (_disposed || _running.containsKey(id) || _queued.containsKey(id)) {
      return;
    }
    _queued[id] = _CarouselImage(id, provider, config);
    _drain();
  }

  void cancel(String id) {
    _queued.remove(id);
    final image = _running[id];
    if (image == null) return;
    image.cancelled = true;
    image.nextFrame?.dispose();
    image.nextFrame = null;
    if (image.receivedFrame) {
      image.detach();
      if (!image.encoding) _finish(image);
    }
  }

  void _drain() {
    if (_draining || _disposed) return;
    _draining = true;
    try {
      while (_running.length < 4 && _queued.isNotEmpty) {
        final image = _queued.remove(_queued.keys.first)!;
        _running[image.id] = image;
        image.stream = image.provider.resolve(image.config);
        image.listener = ImageStreamListener(
          (frame, _) {
            image.receivedFrame = true;
            if (image.cancelled || _disposed) {
              frame.dispose();
              _finish(image);
              return;
            }
            image.nextFrame?.dispose();
            image.nextFrame = frame;
            if (!image.encoding) unawaited(_sendFrames(image));
          },
          onError: (Object _, StackTrace? _) {
            image.receivedFrame = true;
            if (!image.cancelled && !_disposed) {
              unawaited(_send(image.id, null));
            }
            image.cancelled = true;
            image.detach();
            if (!image.encoding) _finish(image);
          },
        );
        image.stream!.addListener(image.listener!);
      }
    } finally {
      _draining = false;
    }
  }

  Future<void> _sendFrames(_CarouselImage image) async {
    image.encoding = true;
    try {
      while (!_disposed && !image.cancelled && image.nextFrame != null) {
        final frame = image.nextFrame!;
        image.nextFrame = null;
        try {
          final bytes = await frame.image.toByteData(
            format: ui.ImageByteFormat.png,
          );
          if (!_disposed && !image.cancelled) {
            await _send(image.id, bytes?.buffer.asUint8List());
          }
        } finally {
          frame.dispose();
        }
      }
    } catch (_) {
      if (!_disposed && !image.cancelled) await _send(image.id, null);
      image.cancelled = true;
    } finally {
      image.encoding = false;
      if (_disposed || image.cancelled) _finish(image);
    }
  }

  Future<void> _send(String id, Uint8List? bytes) async {
    try {
      await channel.invokeMethod<void>('imageFrame', {
        'id': id,
        'bytes': bytes,
      });
    } on PlatformException {
      // The native view may disappear while a frame crosses the bridge.
    } on MissingPluginException {
      // Disposal can remove the channel before an in-flight frame arrives.
    }
  }

  void _finish(_CarouselImage image) {
    image.detach();
    image.nextFrame?.dispose();
    image.nextFrame = null;
    _running.remove(image.id);
    _drain();
  }

  void dispose() {
    _disposed = true;
    _queued.clear();
    for (final image in _running.values.toList()) {
      image.cancelled = true;
      _finish(image);
    }
  }
}

class _CarouselImage {
  _CarouselImage(this.id, this.provider, this.config);
  final String id;
  final ImageProvider<Object> provider;
  final ImageConfiguration config;
  ImageStream? stream;
  ImageStreamListener? listener;
  ImageInfo? nextFrame;
  bool cancelled = false;
  bool receivedFrame = false;
  bool encoding = false;

  void detach() {
    final listener = this.listener;
    if (listener != null) stream?.removeListener(listener);
    this.listener = null;
  }
}
