import 'dart:math' as math;

import 'package:flutter/material.dart';

/// Maximum inline media height shared by images and their placeholders.
const messageMediaMaxImageHeight = 240.0;

/// Width reserved by inline message media at this viewport size.
double messageMediaMaxWidth(BuildContext context) =>
    math.min(MediaQuery.sizeOf(context).width * 0.72, 320.0);

/// Metadata-based image bounds shared by decoded previews and skeletons.
Size messageImagePreviewSize(BuildContext context, double? aspectRatio) {
  final width = messageMediaMaxWidth(context);
  if (aspectRatio == null || !aspectRatio.isFinite || aspectRatio <= 0) {
    return Size(width, messageMediaMaxImageHeight);
  }
  final ratio = aspectRatio.clamp(0.2, 4.0).toDouble();
  final height = math.min(width / ratio, messageMediaMaxImageHeight);
  return Size(height * ratio, height);
}

/// Inline video ratio shared by decoded previews and skeletons.
double messageVideoAspectRatio(double? aspectRatio) =>
    (aspectRatio ?? 16 / 9).clamp(0.75, 1.91).toDouble();
