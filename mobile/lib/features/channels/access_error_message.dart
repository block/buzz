import 'dart:async';

/// A readable access failure without Dart exception type prefixes.
String accessErrorMessage(Object error) {
  if (error is TimeoutException) {
    return 'The request timed out. Check your connection and try again.';
  }
  return error.toString().replaceFirst(
    RegExp(r'^(Bad state: |Exception: |FormatException: )'),
    '',
  );
}
