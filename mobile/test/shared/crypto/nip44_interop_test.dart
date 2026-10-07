import 'package:buzz/shared/crypto/nip44.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('decrypts a desktop nostr-rs NIP-44 v2 self-encrypted payload', () {
    const privateKey =
        '0000000000000000000000000000000000000000000000000000000000000001';
    const publicKey =
        '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798';
    const desktopCiphertext =
        'Au0C/BZ3gT83RnPFiPYGr70BuEyKDlZrk1nEJUDZbkoNgpSjE7JUKRb3VRbegcQYUvNT2Qayf3DkfuSb1M6l70IDpsQ25y8xwDA+uEreyRxDdZ5tQF+C9iB3Qr0vinFQpbR9f0SIvUahwAzyHBMdZ1butlCHi9aqv0C1/w1MWMWeoGaPm4XtkhJSPawCGMuFVw1Z8r64bxMSI6EThc4HtR9p4Q==';

    expect(
      nip44Decrypt(
        getConversationKey(privateKey, publicKey),
        desktopCiphertext,
      ),
      '{"version":1,"theme":"catppuccin-latte","accent":"#f97316","followSystem":false}',
    );
  });
}
