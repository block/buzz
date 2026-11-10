import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Uses Chromium's WebM alpha compositing for a bundled Android avatar.
class AndroidTransparentAvatar extends HookWidget {
  const AndroidTransparentAvatar({
    required this.url,
    required this.playing,
    required this.fallback,
    super.key,
  });

  final Uri url;
  final bool playing;
  final Widget fallback;

  @override
  Widget build(BuildContext context) {
    final channel = useState<MethodChannel?>(null);
    final failed = useState(false);
    final ready = useState(false);
    useEffect(() {
      final current = channel.value;
      if (current == null || !playing || ready.value || failed.value) {
        return null;
      }
      var active = true;
      Timer? retry;
      Future<void> waitForFrame() async {
        try {
          final state = await current.invokeMethod<String>('isReady');
          if (!active) return;
          if (state == 'failed') {
            failed.value = true;
          } else if (state == 'ready') {
            ready.value = true;
          } else {
            retry = Timer(const Duration(milliseconds: 50), waitForFrame);
          }
        } on PlatformException {
          if (active) failed.value = true;
        } on MissingPluginException {
          if (active) failed.value = true;
        }
      }

      unawaited(waitForFrame());
      return () {
        active = false;
        retry?.cancel();
      };
    }, [channel.value, playing, failed.value]);
    useEffect(() {
      final current = channel.value;
      var active = true;
      Future<void> updatePlayback() async {
        try {
          await current?.invokeMethod<void>('setPlaying', playing);
        } on PlatformException {
          if (active) failed.value = true;
        } on MissingPluginException {
          if (active) failed.value = true;
        }
      }

      unawaited(updatePlayback());
      return () => active = false;
    }, [channel.value, playing]);
    if (failed.value) return fallback;
    return ExcludeSemantics(
      child: IgnorePointer(
        child: Stack(
          fit: StackFit.expand,
          children: [
            Opacity(
              opacity: ready.value ? 1 : 0,
              child: AndroidView(
                viewType: 'buzz/transparent-avatar',
                creationParams: {'url': url.toString()},
                creationParamsCodec: const StandardMessageCodec(),
                onPlatformViewCreated: (id) {
                  if (context.mounted) {
                    channel.value = MethodChannel(
                      'buzz/transparent-avatar/$id',
                    );
                  }
                },
              ),
            ),
            IgnorePointer(
              child: AnimatedOpacity(
                opacity: ready.value ? 0 : 1,
                duration: const Duration(milliseconds: 120),
                child: fallback,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
