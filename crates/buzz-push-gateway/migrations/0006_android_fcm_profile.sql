-- Register the Firebase App Check + FCM Android application profile. Existing
-- App Attest columns store the profile-specific installation credential id and
-- P-256 public key; their constraints already cover Android key material.
ALTER TABLE push_gateway_installations
    DROP CONSTRAINT push_gateway_installations_app_profile_check;
ALTER TABLE push_gateway_installations
    ADD CONSTRAINT push_gateway_installations_app_profile_check
    CHECK (app_profile IN ('buzz-ios-dogfood', 'buzz-android-fcm'));
