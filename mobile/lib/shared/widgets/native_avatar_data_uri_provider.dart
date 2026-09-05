import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/media_auth.dart';
import '../relay/media_image.dart';

/// Resolves an avatar through Buzz's authenticated media path and converts it
/// to a small PNG that UIKit can render inside native glass controls and menus.
final nativeAvatarDataUriProvider = FutureProvider.autoDispose
    .family<String?, String>((ref, source) async {
      final trimmed = source.trim();
      if (trimmed.isEmpty) return null;

      final Uint8List sourceBytes;
      if (trimmed.startsWith('data:image/')) {
        try {
          final data = UriData.parse(trimmed);
          if (data.mimeType == 'image/svg+xml') return null;
          if (data.contentAsBytes().isEmpty) return null;
          return trimmed;
        } on FormatException {
          return null;
        }
      } else {
        final uri = Uri.tryParse(trimmed);
        if (uri == null || (uri.scheme != 'http' && uri.scheme != 'https')) {
          return null;
        }
        try {
          final response = await ref
              .watch(mediaHttpClientProvider)
              .get(
                uri,
                headers: ref
                    .watch(mediaGetAuthServiceProvider)
                    .headersFor(trimmed),
              )
              .timeout(const Duration(seconds: 5));
          if (response.statusCode != 200 || response.bodyBytes.isEmpty) {
            return null;
          }
          sourceBytes = response.bodyBytes;
        } catch (_) {
          return null;
        }
      }

      ui.Codec? codec;
      ui.Image? image;
      try {
        codec = await ui.instantiateImageCodec(
          sourceBytes,
          targetWidth: 72,
          targetHeight: 72,
          allowUpscaling: false,
        );
        final frame = await codec.getNextFrame();
        image = frame.image;
        final pngData = await image.toByteData(format: ui.ImageByteFormat.png);
        if (pngData == null) return null;
        final png = pngData.buffer.asUint8List(
          pngData.offsetInBytes,
          pngData.lengthInBytes,
        );
        return 'data:image/png;base64,${base64Encode(png)}';
      } catch (_) {
        return null;
      } finally {
        image?.dispose();
        codec?.dispose();
      }
    });
