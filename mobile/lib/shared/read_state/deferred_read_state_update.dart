import 'package:flutter/widgets.dart';

VoidCallback deferReadStateUpdate(BuildContext context, VoidCallback update) {
  var cancelled = false;
  WidgetsBinding.instance.addPostFrameCallback((_) {
    if (!cancelled && context.mounted) {
      update();
    }
  });

  return () {
    cancelled = true;
  };
}
