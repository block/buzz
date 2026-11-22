import 'relay_session_types.dart';
import 'relay_socket.dart';

/// The text of [error] for showing to the person, without the `Exception: `
/// a plain [Exception] adds. A relay rejection keeps its message verbatim,
/// and a lost acknowledgement drops the prefix only from its cause. Named
/// types keep their whole name: `TimeoutException: ...` stays as is.
///
/// Use this where any error can reach the person and its text should stay
/// whole. For a relay refusal reworded as a sentence (NIP-01 prefix dropped,
/// first letter capitalized), use `relayErrorMessage` instead.
String relayErrorText(Object error) {
  if (error is RelayEventRejectedException) return error.message;
  final text = error.toString();
  final cause = error is RelayAckLostError ? error.cause : null;
  if (cause != null) {
    final causeText = cause.toString();
    if (text.endsWith(causeText)) {
      return text.substring(0, text.length - causeText.length) +
          relayErrorText(cause);
    }
  }
  const prefix = 'Exception: ';
  return text.startsWith(prefix) ? text.substring(prefix.length) : text;
}
