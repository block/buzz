/// Compile-time iOS push capability. It is false for every normal build and is
/// injected as a Dart define only by the tracked PushEnabled.xcconfig overlay.
const buzzPushCapabilityEnabled = bool.fromEnvironment(
  'BUZZ_PUSH_ENABLED',
  defaultValue: false,
);
