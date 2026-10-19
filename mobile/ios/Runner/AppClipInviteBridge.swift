import Flutter
import Foundation

/// Hands the invite parked by the App Clip to Dart.
///
/// `takePendingInvite` returns the stored invocation URL (or nil) and clears
/// every copy in the same call, so the invite is replayed at most once. Dart
/// feeds the URL through the same parser a universal link uses; this bridge
/// never interprets it.
final class BuzzAppClipInviteBridge {
  private let channel: FlutterMethodChannel
  private let store: BuzzPendingInviteStore

  init(messenger: FlutterBinaryMessenger, store: BuzzPendingInviteStore) {
    channel = FlutterMethodChannel(name: "buzz/app_clip_invite", binaryMessenger: messenger)
    self.store = store
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else {
        result(FlutterError(code: "unavailable", message: nil, details: nil))
        return
      }
      guard call.method == "takePendingInvite" else {
        result(FlutterMethodNotImplemented)
        return
      }
      do {
        result(try self.store.take()?.url)
      } catch {
        // A corrupt or unreadable record cannot be retried into validity;
        // drop it so the next launch does not fail the same way.
        self.store.clear()
        result(FlutterError(code: "read_failed", message: error.localizedDescription, details: nil))
      }
    }
  }
}
