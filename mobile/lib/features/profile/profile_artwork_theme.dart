import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/app_theme.dart';
import '../../shared/theme/utility_surface_theme.dart';
import '../../shared/widgets/avatar_image.dart';

ThemeData useProfileArtworkTheme(
  BuildContext context,
  WidgetRef ref,
  String? url, {
  bool personProfile = true,
}) {
  final ambient = Theme.of(context);
  final enabled =
      ambient.platform == TargetPlatform.iOS && url != null && url.isNotEmpty;
  final auth = enabled ? ref.watch(mediaGetAuthServiceProvider) : null;
  final client = enabled ? ref.watch(mediaHttpClientProvider) : null;
  final cache = enabled ? ref.watch(mediaDiskCacheProvider) : null;
  final sample = useMemoized(() async {
    if (auth == null || client == null) return null;
    const side = 32;
    final pixels = await nativeAvatarImage(
      url: url,
      initial: '',
      dimension: side.toDouble(),
      background: Colors.transparent,
      foreground: Colors.white,
      format: ui.ImageByteFormat.rawRgba,
      networkImages: (source) => smallMediaImageProviders(
        url: source,
        auth: auth,
        client: client,
        diskCache: cache,
      ),
    );
    if (pixels == null) return null;
    var red = 0, green = 0, blue = 0, count = 0;
    // Average opaque pixels near the foot of the portrait. Ignore the
    // transparent corners introduced by the avatar mask.
    for (var y = 25; y < side; y++) {
      for (var x = 0; x < side; x++) {
        final i = (y * side + x) * 4;
        if (pixels[i + 3] < 240) continue;
        red += pixels[i];
        green += pixels[i + 1];
        blue += pixels[i + 2];
        count++;
      }
    }
    if (count == 0) return null;
    return Color.fromARGB(255, red ~/ count, green ~/ count, blue ~/ count);
  }, [url, auth, client, cache]);
  final sampled = useFuture(sample, preserveState: false).data;
  if (!enabled) return utilitySurfaceThemeData(ambient);
  if (sampled == null) return utilitySurfaceThemeData(ambient);
  if (personProfile) {
    final lightInk =
        ThemeData.estimateBrightnessForColor(sampled) == Brightness.dark;
    final base = utilitySurfaceThemeData(
      lightInk ? AppTheme.dark() : AppTheme.light(),
    ).copyWith(platform: ambient.platform);
    final ink = lightInk ? Colors.white : Colors.black;
    return base.copyWith(
      scaffoldBackgroundColor: sampled,
      colorScheme: base.colorScheme.copyWith(
        surface: sampled,
        onSurface: ink,
        onSurfaceVariant: lightInk
            ? Color.alphaBlend(ink.withValues(alpha: .8), sampled)
            : profileDarkInk(sampled),
      ),
    );
  }
  final lightMode = ambient.brightness == Brightness.light;
  final lightInk = !lightMode && sampled.computeLuminance() < .18;
  final base = utilitySurfaceThemeData(
    lightInk ? AppTheme.dark() : AppTheme.light(),
  ).copyWith(platform: ambient.platform);
  final surface = lightMode
      ? Color.lerp(base.scaffoldBackgroundColor, sampled, .20)!
      : sampled;
  return base.copyWith(
    scaffoldBackgroundColor: surface,
    colorScheme: base.colorScheme.copyWith(surface: surface),
  );
}

// Deepen the sampled hue while retaining contrast for small lettering.
Color profileDarkInk(Color surface) {
  final hue = HSLColor.fromColor(surface);
  if (hue.saturation < .015) return Colors.black;
  for (final lightness in const [.17, .12, .08]) {
    final ink = hue.withSaturation(.85).withLightness(lightness).toColor();
    if ((surface.computeLuminance() + .05) / (ink.computeLuminance() + .05) >=
        4.5) {
      return ink;
    }
  }
  return Colors.black;
}
