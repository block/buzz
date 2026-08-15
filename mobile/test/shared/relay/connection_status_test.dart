import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const tailnetUrl = 'https://matthews-macbook-pro-1.tailf29f2c.ts.net/';

  test('recognizes a tailnet relay by its DNS suffix', () {
    expect(isTailnetRelayUrl(tailnetUrl), isTrue);
    expect(isTailnetRelayUrl('https://relay.example'), isFalse);
  });

  test('describes a connected tailnet community as private', () {
    final presentation = relayConnectionPresentation(
      tailnetUrl,
      const SessionState(status: SessionStatus.connected),
    );

    expect(presentation.title, 'Connected privately');
    expect(presentation.detail, tailnetUrl);
  });

  test('gives a Tailscale recovery action for a private network failure', () {
    final presentation = relayConnectionPresentation(
      tailnetUrl,
      const SessionState(
        status: SessionStatus.reconnecting,
        failureKind: SessionFailureKind.network,
      ),
    );

    expect(presentation.title, 'Private relay unavailable');
    expect(presentation.detail, 'Check Tailscale or VPN');
  });

  test('gives re-pair guidance for an authentication rejection', () {
    final presentation = relayConnectionPresentation(
      tailnetUrl,
      const SessionState(
        status: SessionStatus.disconnected,
        failureKind: SessionFailureKind.authentication,
      ),
    );

    expect(presentation.title, 'Authentication failed');
    expect(presentation.detail, 'Re-pair this community from Command Adviser');
  });

  test('does not mention Tailscale for an ordinary relay failure', () {
    final presentation = relayConnectionPresentation(
      'https://relay.example',
      const SessionState(
        status: SessionStatus.reconnecting,
        failureKind: SessionFailureKind.network,
      ),
    );

    expect(presentation.title, 'Relay unavailable');
    expect(presentation.detail, 'Check your internet connection');
    expect(
      '${presentation.title} ${presentation.detail}',
      isNot(contains('Tailscale')),
    );
  });
}
