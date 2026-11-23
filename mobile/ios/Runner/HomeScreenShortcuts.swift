import Flutter
import UIKit

/// Publishes Home Screen quick actions from Flutter and hands taps back to it.
///
/// A tap can arrive before Dart is listening (cold launch, or before Home
/// mounts), so until Dart first takes the buffered tap the latest one waits
/// there. After that, each tap is sent to Dart once.
final class HomeScreenShortcuts {
  static let shared = HomeScreenShortcuts()

  private static let types: Set<String> = ["new-chat", "new-dm", "message-bestie"]
  private static let icons: [String: UIApplicationShortcutIcon] = [
    "new-chat": UIApplicationShortcutIcon(systemImageName: "square.and.pencil"),
    "new-dm": UIApplicationShortcutIcon(systemImageName: "bubble.left"),
    "message-bestie": UIApplicationShortcutIcon(templateImageName: "ShortcutAgent"),
  ]

  private var channel: FlutterMethodChannel?
  private var pending: String?
  /// Whether Dart has taken the buffered tap and now receives taps directly.
  private var listening = false

  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "buzz/home_screen_shortcuts", binaryMessenger: messenger)
    self.channel = channel
    channel.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "setItems":
        guard let items = call.arguments as? [[String: String]] else {
          result(FlutterError(code: "invalid_input", message: nil, details: nil))
          return
        }
        var shortcuts: [UIApplicationShortcutItem] = []
        for item in items {
          guard let type = item["type"], Self.types.contains(type), let title = item["title"] else {
            result(FlutterError(code: "invalid_input", message: "Unknown shortcut", details: item))
            return
          }
          shortcuts.append(
            UIApplicationShortcutItem(
              type: type, localizedTitle: title, localizedSubtitle: nil, icon: Self.icons[type], userInfo: nil))
        }
        UIApplication.shared.shortcutItems = shortcuts
        result(nil)
      case "takePending":
        result(self?.pending)
        self?.pending = nil
        self?.listening = true
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  /// Returns false for items this bridge does not own.
  @discardableResult
  func handle(_ item: UIApplicationShortcutItem) -> Bool {
    guard Self.types.contains(item.type) else { return false }
    let type = item.type
    pending = type
    // Sending a tap that is also buffered could reach Dart twice: once from
    // the channel's own buffer and once from takePending.
    guard listening else { return true }
    pending = nil
    channel?.invokeMethod("open", arguments: type)
    return true
  }
}
