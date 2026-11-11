import Flutter
import UIKit

/// Publishes Home Screen quick actions from Flutter and hands taps back to it.
///
/// A tap can arrive before Dart installs its handler (cold launch) or while
/// Dart is not listening, so the latest tap is buffered until Dart takes it or
/// acknowledges a warm delivery.
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
    channel?.invokeMethod("open", arguments: type) { [weak self] result in
      guard result as? String == "handled", self?.pending == type else { return }
      self?.pending = nil
    }
    return true
  }
}
