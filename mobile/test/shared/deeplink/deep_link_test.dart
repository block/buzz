import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  _inviteTests();

  group('parseMessageDeepLink', () {
    test('parses channel and id', () {
      final link = parseMessageDeepLink(
        Uri.parse('buzz://message?channel=d14cd131&id=abc123'),
      );
      expect(
        link,
        const MessageDeepLink(channelId: 'd14cd131', messageId: 'abc123'),
      );
    });

    test('parses optional thread param', () {
      final link = parseMessageDeepLink(
        Uri.parse('buzz://message?channel=d14cd131&id=abc123&thread=root99'),
      );
      expect(link?.threadRootId, 'root99');
    });

    test('treats empty thread as absent', () {
      final link = parseMessageDeepLink(
        Uri.parse('buzz://message?channel=d14cd131&id=abc123&thread='),
      );
      expect(link, isNotNull);
      expect(link?.threadRootId, isNull);
    });

    test('rejects missing channel', () {
      expect(parseMessageDeepLink(Uri.parse('buzz://message?id=abc')), isNull);
    });

    test('rejects empty channel', () {
      expect(
        parseMessageDeepLink(Uri.parse('buzz://message?channel=&id=abc')),
        isNull,
      );
    });

    test('rejects missing id', () {
      expect(
        parseMessageDeepLink(Uri.parse('buzz://message?channel=d14cd131')),
        isNull,
      );
    });

    test('rejects non-buzz scheme', () {
      expect(
        parseMessageDeepLink(Uri.parse('https://message?channel=a&id=b')),
        isNull,
      );
    });

    test('rejects non-message host (connect is desktop-only)', () {
      expect(
        parseMessageDeepLink(Uri.parse('buzz://connect?relay=wss://x')),
        isNull,
      );
    });
  });
}

void _inviteTests() {
  group('parseInviteDeepLink', () {
    test('parses canonical HTTPS invite URL', () {
      final link = parseInviteDeepLink(
        Uri.parse('https://relay.example.com/invite/abc123'),
      );
      expect(
        link,
        const InviteDeepLink(
          relayUrl: 'wss://relay.example.com',
          code: 'abc123',
        ),
      );
    });

    test('parses HTTP invite URL for local/dev relays', () {
      final link = parseInviteDeepLink(
        Uri.parse('http://localhost:3000/invite/dev-code'),
      );
      expect(
        link,
        const InviteDeepLink(relayUrl: 'ws://localhost:3000', code: 'dev-code'),
      );
    });

    test('parses buzz join handoff link', () {
      final link = parseInviteDeepLink(
        Uri.parse(
          'buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=abc123',
        ),
      );
      expect(
        link,
        const InviteDeepLink(
          relayUrl: 'wss://relay.example.com',
          code: 'abc123',
        ),
      );
    });

    test('preserves policy receipt in buzz join handoff', () {
      final link = parseInviteDeepLink(
        Uri.parse(
          'buzz://join?relay=wss%3A%2F%2Frelay.example.com&code=abc123&policy_receipt=receipt.value',
        ),
      );
      expect(
        link,
        const InviteDeepLink(
          relayUrl: 'wss://relay.example.com',
          code: 'abc123',
          policyReceipt: 'receipt.value',
        ),
      );
    });

    test('rejects non-invite HTTPS paths', () {
      expect(
        parseInviteDeepLink(Uri.parse('https://relay.example.com/api/invites')),
        isNull,
      );
      expect(
        parseInviteDeepLink(Uri.parse('https://relay.example.com/invite/')),
        isNull,
      );
      expect(
        parseInviteDeepLink(Uri.parse('https://relay.example.com/invite/a/b')),
        isNull,
      );
    });

    test('rejects credentials and fragments', () {
      expect(
        parseInviteDeepLink(
          Uri.parse('https://user:pass@relay.example.com/invite/abc'),
        ),
        isNull,
      );
      expect(
        parseInviteDeepLink(
          Uri.parse('https://relay.example.com/invite/abc#x'),
        ),
        isNull,
      );
      expect(
        parseInviteDeepLink(
          Uri.parse(
            'buzz://join?relay=wss%3A%2F%2Fuser%3Apass%40relay.example.com&code=abc',
          ),
        ),
        isNull,
      );
    });

    test('rejects buzz join without websocket relay or code', () {
      expect(
        parseInviteDeepLink(
          Uri.parse('buzz://join?relay=https://relay.example.com&code=abc'),
        ),
        isNull,
      );
      expect(
        parseInviteDeepLink(
          Uri.parse('buzz://join?relay=wss://relay.example.com'),
        ),
        isNull,
      );
      expect(
        parseInviteDeepLink(Uri.parse('buzz://connect?relay=wss://x')),
        isNull,
      );
    });

    test('rejects buzz join with dangerous relay schemes', () {
      // The `relay=` param is an allowlist — only `ws` / `wss` are safe to
      // hand to a Nostr relay session. Anything else must be dropped by the
      // parser so a hostile QR / share link can't smuggle a browser scheme
      // (`javascript:`, `data:`), a local resource (`file:`), or an
      // unrelated transport (`ftp:`, `chrome:`) into the join flow.
      for (final hostile in [
        'javascript:alert(1)',
        'data:text/html,evil',
        'file:///etc/passwd',
        'ftp://relay.example.com',
        'chrome://settings',
        'about:blank',
        'ssh://relay.example.com',
      ]) {
        final encoded = Uri.encodeQueryComponent(hostile);
        expect(
          parseInviteDeepLink(Uri.parse('buzz://join?relay=$encoded&code=abc')),
          isNull,
          reason: 'must reject relay scheme in $hostile',
        );
      }
    });
  });
}
