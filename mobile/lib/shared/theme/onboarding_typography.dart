import 'package:flutter/material.dart';

/// Shared setup-page typography, using the approved Inter treatment.
abstract final class OnboardingTypography {
  static const title = TextStyle(
    fontFamily: 'Inter',
    fontSize: 30,
    height: 36 / 30,
    fontWeight: FontWeight.w400,
    letterSpacing: -1,
  );
  static const subtitle = TextStyle(
    fontFamily: 'Inter',
    fontSize: 16,
    height: 24 / 16,
    fontWeight: FontWeight.w400,
    letterSpacing: -0.25,
  );
}
