#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_root"

fail() {
  printf 'FAIL %s\n' "$1" >&2
  exit 1
}

expect_contains() {
  file=$1
  value=$2
  label=$3
  grep -Fq "$value" "$file" || fail "$label"
}

expect_absent() {
  file=$1
  value=$2
  label=$3
  if grep -Fq "$value" "$file"; then
    fail "$label"
  fi
}

project=mobile/ios/Runner.xcodeproj/project.pbxproj
runner_entitlements=mobile/ios/Runner/Runner.entitlements
extension_entitlements=mobile/ios/NotificationService/NotificationService.entitlements
runner_info=mobile/ios/Runner/Info.plist
extension_info=mobile/ios/NotificationService/Info.plist
app_delegate=mobile/ios/Runner/AppDelegate.swift
notification_service=mobile/ios/NotificationService/NotificationService.swift
template=mobile/ios/Flutter/AppOverrides.xcconfig.example

family_count=$(grep -Fc 'TARGETED_DEVICE_FAMILY = "1,2";' "$project" || true)
[ "$family_count" -eq 9 ] || fail "all Apple build configurations must target iPhone and iPad"

extension_bundle_count=$(
  grep -Fc 'PRODUCT_BUNDLE_IDENTIFIER = "$(BUNDLE_IDENTIFIER).NotificationService";' "$project" || true
)
[ "$extension_bundle_count" -eq 3 ] || fail "the extension bundle identifier must remain derived"

expect_contains "$runner_entitlements" '$(AppIdentifierPrefix)$(BUZZ_APP_KEYCHAIN_ACCESS_GROUP)' \
  "Runner must retain an app-only Keychain group"
expect_contains "$runner_entitlements" '$(AppIdentifierPrefix)$(BUZZ_EXTENSION_KEYCHAIN_ACCESS_GROUP)' \
  "Runner must share only the extension owner-key group"
expect_contains "$extension_entitlements" '$(AppIdentifierPrefix)$(BUZZ_EXTENSION_KEYCHAIN_ACCESS_GROUP)' \
  "the extension must have its dedicated owner-key group"
expect_absent "$extension_entitlements" 'BUZZ_APP_KEYCHAIN_ACCESS_GROUP' \
  "the extension must not have the app-only Keychain group"
expect_absent "$extension_entitlements" 'aps-environment' \
  "the extension must not carry the containing app push entitlement"
expect_absent "$extension_entitlements" 'com.apple.developer.devicecheck.appattest-environment' \
  "the extension must not carry App Attest"
expect_absent "$extension_entitlements" 'com.apple.developer.usernotifications.communication' \
  "the extension must not carry the containing app communication entitlement"

expect_contains "$runner_info" '<key>BuzzAppKeychainAccessGroup</key>' \
  "Runner must expose its app-only Keychain group"
expect_contains "$runner_info" '<key>BuzzExtensionKeychainAccessGroup</key>' \
  "Runner must expose its extension Keychain group"
expect_contains "$extension_info" '<key>BuzzExtensionKeychainAccessGroup</key>' \
  "the extension must expose only its extension Keychain group"
expect_absent "$extension_info" '<key>BuzzAppKeychainAccessGroup</key>' \
  "the extension must not expose the app-only Keychain group"

expect_contains "$app_delegate" 'accessGroup: appKeychainAccessGroup' \
  "gateway grants must use the app-only Keychain group"
expect_contains "$app_delegate" 'appAttestKeychainAccessGroup: appKeychainAccessGroup' \
  "App Attest state must use the app-only Keychain group"
expect_contains "$app_delegate" 'keychainAccessGroup: extensionKeychainAccessGroup' \
  "only the resolver owner identity may use the extension Keychain group"
expect_contains "$notification_service" \
  'forInfoDictionaryKey: "BuzzExtensionKeychainAccessGroup"' \
  "the extension resolver must use only its dedicated Keychain group"
expect_absent "$notification_service" 'BuzzAppKeychainAccessGroup' \
  "the extension must not address app-only Keychain state"

expect_contains mobile/ios/Runner/PushNativeState.swift \
  'kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly' \
  "extension owner keys must remain device-bound"
expect_contains mobile/scripts/require-push-gateway-origin.sh \
  'BUZZ_PUSH_APP_PROFILE' \
  "the Xcode build gate must propagate the selected app profile"

encoded_profile=$(
  env -u BUZZ_PUSH_GATEWAY_URL -u DART_DEFINES \
    BUZZ_PUSH_APP_PROFILE=buzz-ios-custom \
    /bin/sh -c '. mobile/scripts/require-push-gateway-origin.sh; printf "%s" "$DART_DEFINES"'
)
decoded_profile=$(printf '%s' "$encoded_profile" | base64 --decode 2>/dev/null \
  || printf '%s' "$encoded_profile" | base64 -D 2>/dev/null)
[ "$decoded_profile" = 'BUZZ_PUSH_APP_PROFILE=buzz-ios-custom' ] \
  || fail "the custom Xcode app profile must reach Flutter unchanged"

expect_contains "$template" 'BUZZ_PUSH_APP_PROFILE = buzz-ios-custom' \
  "the custom template must select the custom app profile"
expect_contains "$template" 'BUZZ_IOS_PUSH_ENVIRONMENT = production' \
  "the custom template must select production APNs"
expect_contains "$template" 'BUZZ_APP_ATTEST_ENVIRONMENT = production' \
  "the custom template must select production App Attest"
expect_contains "$template" 'REPLACE_WITH_YOUR_BUNDLE_IDENTIFIER' \
  "the custom template must not contain a real bundle identifier"
expect_contains "$template" 'REPLACE_WITH_YOUR_TEAM_ID' \
  "the custom template must not contain a real team identifier"
expect_absent "$template" 'xyz.block' \
  "the custom template must not contain the upstream bundle identity"
expect_absent "$template" 'group.xyz' \
  "the custom template must not contain the upstream App Group"

printf 'PASS iOS production enrollment shell is universal and isolated\n'
