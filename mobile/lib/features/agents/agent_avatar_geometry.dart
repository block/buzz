import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';

/// Space reserved below previews for the filmstrip.
const agentAvatarTraySpace = 115.0;

/// Preview space added above the editor controls.
const agentAvatarPreviewSpace = 228.0;

/// Maximum character preview size.
const agentAvatarMaxPreviewSize = 440.0;

/// Asset padding compensation shared by previews and Hero flights.
double agentCharacterOffset(String? url, {double scale = 1}) =>
    url?.contains('/buzzies/') == true ? -0.084 * scale : 0;

/// Shrinks only items near clipped filmstrip edges.
double agentFilmstripScale({
  required int index,
  required int count,
  required double extent,
  required double width,
  required double offset,
  required bool reduceMotion,
}) {
  if (reduceMotion) return 1;
  final center = Grid.xxs + index * extent + extent / 2 - offset;
  final maxScroll = math.max(0.0, count * extent + Grid.xxs * 2 - width);
  final distance = math.min(
    center + math.max(0.0, 72 - offset),
    width - center + math.max(0.0, 72 - (maxScroll - offset)),
  );
  return 0.65 +
      0.35 * Curves.easeInOut.transform((distance / 72).clamp(0.0, 1.0));
}
