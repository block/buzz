import 'package:buzz/shared/deeplink/community_link_input.dart';
import 'package:buzz/shared/deeplink/deep_link.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('normalizes addresses through the existing community parser', () {
    for (final value in [
      ' team.communities.buzz.xyz ',
      'https://team.communities.buzz.xyz/',
      'wss://team.communities.buzz.xyz',
    ]) {
      final uri = parseCommunityLinkInput(value)!;
      final link = parseBuzzDeepLink(uri) as AddCommunityDeepLink;
      expect(link.relayUrl, 'wss://team.communities.buzz.xyz');
    }
  });

  test('preserves the invite and its destination', () {
    final uri = parseCommunityLinkInput(
      'https://community.example.com/invite/abc123',
    )!;
    final link = parseBuzzDeepLink(uri) as InviteDeepLink;
    expect(link.code, 'abc123');
    expect(link.relayUrl, 'wss://community.example.com');
  });

  test('rejects unrelated actions and unsafe or incomplete addresses', () {
    for (final value in [
      '',
      'not a link',
      'http://community.example.com',
      'https://user:secret@community.example.com',
      'https://community.example.com/other',
      'https://community.example.com/invite/',
      'https://community.example.com#secret',
      'https://community.example.com?token=secret',
      'https://127.0.0.1',
      'https://community.example.com.',
      'buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32',
      'nostrpair://example',
    ]) {
      expect(parseCommunityLinkInput(value), isNull, reason: value);
    }
  });
}
