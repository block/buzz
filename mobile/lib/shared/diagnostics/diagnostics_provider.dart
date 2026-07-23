import 'package:hooks_riverpod/legacy.dart';

import 'diagnostics_controller.dart';

/// Created during startup and overridden in main.dart.
final diagnosticsControllerProvider =
    ChangeNotifierProvider<DiagnosticsController>(
      (_) => throw UnimplementedError('Must be overridden'),
    );
