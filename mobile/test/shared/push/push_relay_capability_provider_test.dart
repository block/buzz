import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final (failure, reported) in <(Object, bool)>[
    (const FormatException('malformed descriptor'), true),
    (const BuzzPushNotAdvertised(), false),
    (StateError('relay unreachable'), false),
  ]) {
    test('$failure keeps capability inactive'
        '${reported ? ' and is reported' : ''}', () async {
      final reporter = _RecordingDiagnostics();
      final descriptor = await discoverBuzzPushRelayCapability(
        'https://relay.example',
        fetchDescriptor: (_) async => throw failure,
        diagnostics: reporter,
      );

      expect(descriptor, isNull);
      expect(reporter.operations, [
        if (reported) DiagnosticOperation.pushGatewayResponse,
      ]);
    });
  }

  test('a malformed descriptor that arrives after a switch began is not '
      'reported', () async {
    final reporter = _RecordingDiagnostics();
    final descriptor = await discoverBuzzPushRelayCapability(
      'https://relay.example',
      fetchDescriptor: (_) async =>
          throw const FormatException('malformed descriptor'),
      diagnostics: reporter,
      isCurrent: () => false,
    );

    expect(descriptor, isNull);
    expect(reporter.operations, isEmpty);
  });
}

class _RecordingDiagnostics extends NoopDiagnosticReporter {
  final operations = <DiagnosticOperation>[];

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) => operations.add(operation);
}
