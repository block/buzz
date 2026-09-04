import 'package:flutter/foundation.dart';

/// Whether a message-action backdrop is covering the conversation timeline.
///
/// iOS platform views sit outside Flutter's composited scene, so a Flutter
/// backdrop filter cannot blur them. Glass controls listen to this value and
/// temporarily render their Flutter fallback while the backdrop is visible.
final ValueNotifier<bool> messageActionBackdropActive = ValueNotifier(false);
