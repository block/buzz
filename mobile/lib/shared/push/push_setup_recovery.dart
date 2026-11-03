import 'package:flutter/foundation.dart';

/// Push is optional background work. Retry failures with bounded backoff while
/// keeping the failure observable. After three retries, wait for corrected inputs
/// or an explicit retry instead of looping forever.
Duration? pushSetupRetry(int retryCount, Object error) =>
    retryCount < 3 ? Duration(seconds: 5 * (1 << retryCount)) : null;

/// Reports the failed push operation with its original exception and stack.
void reportPushSetupError(String operation, Object error, StackTrace stack) {
  debugPrint('Push $operation failed: $error');
  debugPrintStack(stackTrace: stack);
}
