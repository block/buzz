import 'dart:async';

import 'package:flutter/material.dart';

import '../../../shared/widgets/buzz_titled_sheet_layout.dart';

typedef MeSettingsPresenter =
    Future<String?> Function(
      String title,
      Widget Function(ValueChanged<String>) builder,
    );

/// Keeps the modal route, native surface and scrim alive across picker pages.
class MeSettingsSheet extends StatefulWidget {
  const MeSettingsSheet({super.key, required this.builder});

  final Widget Function(MeSettingsPresenter present, VoidCallback refresh)
  builder;

  @override
  State<MeSettingsSheet> createState() => _MeSettingsSheetState();
}

class _MeSettingsSheetState extends State<MeSettingsSheet> {
  String? _title;
  Widget? _picker;
  Completer<String?>? _result;

  Future<String?> _present(
    String title,
    Widget Function(ValueChanged<String>) builder,
  ) {
    if (!mounted) return Future.value();
    final result = Completer<String?>();
    setState(() {
      _result = result;
      _title = title;
      _picker = builder(_finish);
    });
    return result.future;
  }

  void _finish([String? value]) {
    final result = _result;
    setState(() {
      _result = null;
      _title = null;
      _picker = null;
    });
    result?.complete(value);
  }

  @override
  void dispose() {
    _result?.complete();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final content =
        _picker ??
        widget.builder(_present, () {
          if (mounted) setState(() {});
        });
    return PopScope(
      canPop: _picker == null,
      onPopInvokedWithResult: (didPop, result) {
        if (!didPop && _picker != null) _finish();
      },
      child: BuzzTitledSheetLayout(
        title: _title ?? 'Chat settings',
        showDragHandle: true,
        onClose: () {
          if (_picker != null) {
            _finish();
          } else {
            Navigator.of(context).pop();
          }
        },
        child: MediaQuery.disableAnimationsOf(context)
            ? content
            : AnimatedSize(
                duration: const Duration(milliseconds: 220),
                curve: Curves.easeOutCubic,
                alignment: Alignment.topCenter,
                child: content,
              ),
      ),
    );
  }
}
