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
      'radius': radius,
      'brightness': brightness,
      'rtl': rtl,
    };
    useEffect(() {
      final bridge = channel.value;
      if (bridge == null) return null;
      var active = true;
      final pending = <VoidCallback>{};
      bridge.setMethodCallHandler((call) async {
        final widget = latest.value;
        final index = call.arguments;
        if (call.method == 'scroll' && index is num) {
          widget.onScroll(index.toDouble());
        } else if (index is int &&
            index >= 0 &&
            index < widget.providers.length) {
          switch (call.method) {
            case 'open':
              widget.onOpen(index);
            case 'page':
              widget.onPageChanged(index);
            case 'image':
              final result = Completer<ImageInfo?>();
              final stream = widget.providers[index].resolve(
                createLocalImageConfiguration(context),
              );
              final listener = ImageStreamListener(
                (image, _) {
                  if (!result.isCompleted) {
                    result.complete(image.clone());
                  }
                  image.dispose();
                },
                onError: (Object _, StackTrace? _) {
                  if (!result.isCompleted) result.complete(null);
                },
              );
              void cancel() {
                stream.removeListener(listener);
                if (!result.isCompleted) result.complete(null);
              }
              pending.add(cancel);
              stream.addListener(listener);
              final image = await result.future.timeout(
                const Duration(seconds: 30),
                onTimeout: () => null,
              );
              cancel();
              pending.remove(cancel);
              if (image == null) return null;
              try {
                if (!active) return null;
                final bytes = await image.image.toByteData(
                  format: ui.ImageByteFormat.png,
                );
                return active ? bytes?.buffer.asUint8List() : null;
              } finally {
                image.dispose();
              }
          }
        }
        return null;
      });
      // UIKit requests visible thumbnails only after the Dart handler exists.
      unawaited(bridge.invokeMethod<void>('configure', configuration));
      return () {
        active = false;
        for (final cancel in pending) {
          cancel();
        }
        bridge.setMethodCallHandler(null);
      };
    }, [channel.value]);
    useEffect(() {
      final bridge = channel.value;
      if (bridge != null) {
        unawaited(bridge.invokeMethod<void>('configure', configuration));
      }
      return null;
    }, [width, gutter, gap, radius, brightness, rtl, labelSignature]);

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
