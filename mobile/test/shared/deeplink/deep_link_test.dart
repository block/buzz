import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  _inviteTests();
  _channelTests();
  _buildMessageLinkTests();

  group('parseMessageDeepLink', () {
    const channel = '580ca78b-9dae-46f3-8854-bd671853ba32';
    const id =
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    const thread =
        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

    test('parses canonical channel, id, and optional thread', () {
      expect(
        parseMessageDeepLink(
          Uri.parse('buzz://message?channel=$channel&id=$id&thread=$thread'),
        ),
        const MessageDeepLink(
          channelId: channel,
          messageId: id,
          threadRootId: thread,
        ),
      );
    });

    test('rejects malformed or ambiguous forms', () {
      for (final url in [
        'buzz://message?id=$id',
        'buzz://message?channel=&id=$id',
        'buzz://message?channel=$channel',
        'https://message?channel=$channel&id=$id',
        'buzz://connect?channel=$channel&id=$id',
        'buzz://message:1234?channel=$channel&id=$id',
        'buzz://message/path?channel=$channel&id=$id',
        'buzz://message?channel=$channel&id=$id#fragment',
        'buzz://user@message?channel=$channel&id=$id',
        'buzz://message?channel=$channel&id=$id&extra=true',
        'buzz://message?channel=$channel&channel=$channel&id=$id',
        'buzz://message?channel=$channel&id=$id&id=$id',
        'buzz://message?channel=$channel&id=$id&thread=',
        'buzz://message?channel=not-a-uuid&id=$id',
        'buzz://message?channel=$channel&id=not-hex',
        'buzz://message?channel=$channel&id=$id&thread=not-hex',
      ]) {
        expect(parseMessageDeepLink(Uri.parse(url)), isNull, reason: url);
      }
    });
  });
}

void _channelTests() {
  group('parseChannelDeepLink', () {
    test('parses canonical channel path', () {
      expect(
        parseChannelDeepLink(
          Uri.parse('buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32'),
        ),
        const ChannelDeepLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
        ),
      );
    });

    test('accepts v7 and canonicalizes uppercase UUIDs', () {
      expect(
        parseChannelDeepLink(
          Uri.parse('buzz://channel/018fdb5d-3a64-7c35-b5f9-4a23e1f9d2d9'),
        ),
        const ChannelDeepLink(
          channelId: '018fdb5d-3a64-7c35-b5f9-4a23e1f9d2d9',
        ),
      );
      expect(
        parseChannelDeepLink(
          Uri.parse('buzz://channel/580CA78B-9DAE-46F3-8854-BD671853BA32'),
        ),
        const ChannelDeepLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
        ),
      );
    });

    test('rejects missing, extra, query, and fragment forms', () {
      for (final url in [
        'buzz://channel',
        'buzz://channel/',
        'buzz://channel/one/two',
        'buzz://channel:1234/580ca78b-9dae-46f3-8854-bd671853ba32',
        'buzz://channel/one?extra=true',
        'buzz://channel/one#fragment',
        'https://channel/one',
        'buzz://channel/not-a-uuid',
        'buzz://channel/%2F',
        'buzz://channel/%00',
      ]) {
        expect(parseChannelDeepLink(Uri.parse(url)), isNull, reason: url);
      }
    });

    test('is included in the top-level parser', () {
      expect(
        parseBuzzDeepLink(
          Uri.parse('buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32'),
        ),
        const ChannelDeepLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
        ),
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

    test('normalizes trailing slash in buzz join handoff', () {
      final link = parseInviteDeepLink(
        Uri.parse(
          'buzz://join?relay=wss%3A%2F%2Frelay.example.com%2F&code=abc123',
        ),
      );
      expect(link?.relayUrl, 'wss://relay.example.com');
    });

    test('rejects plaintext public buzz join handoff', () {
      final relay = Uri.encodeQueryComponent('ws://relay.example.com');
      expect(
        parseInviteDeepLink(Uri.parse('buzz://join?relay=$relay&code=abc')),
        isNull,
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

    test('rejects non-public invite relay destinations', () {
      for (final url in [
        'https://127.0.0.1/invite/abc',
        'https://169.254.169.254/invite/abc',
        'https://192.168.1.1/invite/abc',
        'https://[::1]/invite/abc',
        'https://[::ffff:127.0.0.1]/invite/abc',
      ]) {
        expect(parseInviteDeepLink(Uri.parse(url)), isNull, reason: url);
      }
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

void _buildMessageLinkTests() {
  group('buildMessageLink', () {
    test('builds channel + id link', () {
      expect(
        buildMessageLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          messageId:
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        ),
        'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      );
    });

    test('includes thread root when present', () {
      expect(
        buildMessageLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          messageId:
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          threadRootId:
              'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
        ),
        'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&thread=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      );
    });

    test('treats empty thread root as absent', () {
      expect(
        buildMessageLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          messageId:
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          threadRootId: '',
        ),
        'buzz://message?channel=580ca78b-9dae-46f3-8854-bd671853ba32&id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
      );
    });

    test('round-trips through parseMessageDeepLink', () {
      final url = buildMessageLink(
        channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
        messageId:
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
        threadRootId:
            'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
      );
      final parsed = parseMessageDeepLink(Uri.parse(url));
      expect(
        parsed,
        const MessageDeepLink(
          channelId: '580ca78b-9dae-46f3-8854-bd671853ba32',
          messageId:
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
          threadRootId:
              'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
        ),
      );
    });

    test('throws on empty channel or id', () {
      expect(
        () => buildMessageLink(channelId: '', messageId: 'abc'),
        throwsArgumentError,
      );
      expect(
        () => buildMessageLink(channelId: 'chan', messageId: ''),
        throwsArgumentError,
      );
    });
  });
  group('entity deep links', () {
    final owner = 'ab' * 32;
    final id = 'cd' * 32;

    test('parses repo, PR, and issue permalinks', () {
      expect(
        parseEntityDeepLink(Uri.parse('buzz://repo?owner=$owner&d=buzz'))?.type,
        'repo',
      );
      expect(
        parseEntityDeepLink(
          Uri.parse('buzz://pr?id=$id&owner=$owner&d=buzz'),
        )?.eventId,
        id,
      );
      expect(
        parseEntityDeepLink(
          Uri.parse('buzz://issue?id=$id&owner=$owner&d=buzz'),
        )?.type,
        'issue',
      );
    });

    test('rejects malformed entity permalinks', () {
      expect(
        parseEntityDeepLink(Uri.parse('buzz://repo?owner=short&d=buzz')),
        isNull,
      );
      expect(
        parseEntityDeepLink(
          Uri.parse('buzz://pr?id=$id&owner=$owner&d=buzz&extra=true'),
        ),
        isNull,
      );
      expect(
        parseEntityDeepLink(Uri.parse('buzz://repo?owner=$owner&d=a..b')),
        isNull,
      );
      expect(
        parseEntityDeepLink(
          Uri.parse('buzz://repo?owner=$owner&d=${'a' * 65}'),
        ),
        isNull,
      );
      for (final url in [
        'buzz://repo?owner=$owner&owner=$owner&d=buzz',
        'buzz://repo?owner=$owner&d=buzz&d=other',
        'buzz://pr?id=$id&id=$id&owner=$owner&d=buzz',
        'buzz://issue?id=$id&owner=$owner&owner=$owner&d=buzz',
      ]) {
        expect(parseEntityDeepLink(Uri.parse(url)), isNull, reason: url);
      }
    });
  });
}
