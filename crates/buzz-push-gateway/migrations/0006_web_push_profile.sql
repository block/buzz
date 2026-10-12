-- Admit the browser/PWA Web Push profile alongside the dogfood iOS profile.
-- buzz-web installations store an AEAD-sealed Web Push subscription in
-- token_ciphertext and a 0x02-prefixed member pubkey in app_attest_public_key
-- (ownership proof is NIP-98, not App Attest; the assertion counter stays 0).
ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_profile_check;
ALTER TABLE push_gateway_installations
    ADD CONSTRAINT push_gateway_installations_app_profile_check
    CHECK (app_profile IN ('buzz-ios-dogfood', 'buzz-web'));
