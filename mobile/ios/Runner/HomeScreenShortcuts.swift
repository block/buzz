import Flutter
import UIKit
import WidgetKit

/// Publishes Home Screen quick actions and the lock screen Bestie from
/// Flutter, and hands quick action and widget taps back to it.
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
      case "setBestie":
        guard let args = call.arguments as? [String: Any] else {
          result(FlutterError(code: "invalid_input", message: nil, details: nil))
          return
        }
        let state = BestieWidgetState(
          name: args["name"] as? String,
          avatar: (args["avatar"] as? FlutterStandardTypedData)?.data)
        guard state.save() else {
          result(FlutterError(code: "unavailable", message: "No app group", details: nil))
          return
        }
        WidgetCenter.shared.reloadTimelines(ofKind: BestieWidgetState.kind)
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
    handle(type: item.type)
  }

  /// Handles a lock screen widget tap, `buzz://home-shortcut/<type>`.
  /// Returns false for other URLs.
  func handle(_ url: URL) -> Bool {
    guard url.scheme == "buzz", url.host == "home-shortcut" else { return false }
    return handle(type: url.lastPathComponent)
  }

  private func handle(type: String) -> Bool {
    guard Self.types.contains(type) else { return false }
    pending = type
    channel?.invokeMethod("open", arguments: type) { [weak self] result in
      guard result as? String == "handled", self?.pending == type else { return }
      self?.pending = nil
    }
    return true
  }
}
