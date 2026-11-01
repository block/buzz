import 'package:flutter/material.dart';

/// Keeps transient feedback above the entire app, including navigator routes
/// and app-level chrome. Install in MaterialApp.builder outside the app shell.
class BuzzToastHost extends StatelessWidget {
  const BuzzToastHost({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Overlay.wrap(child: child);
}
