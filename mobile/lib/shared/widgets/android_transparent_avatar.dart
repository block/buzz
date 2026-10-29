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
        child: AndroidView(
          viewType: 'buzz/transparent-avatar',
          creationParams: {'url': url.toString()},
          creationParamsCodec: const StandardMessageCodec(),
          onPlatformViewCreated: (id) {
            if (context.mounted) {
              channel.value = MethodChannel('buzz/transparent-avatar/$id');
            }
          },
        ),
      ),
    );
  }
}
