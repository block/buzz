import Foundation

/// Links that run a Home Screen quick action, for example
/// `buzz://shortcut/new-chat`. Built into both Runner and BuzzControls, whose
/// Lock Screen widgets open these links.
enum ShortcutLink {
  static func url(_ type: String) -> URL? {
    URL(string: "buzz://shortcut/\(type)")
  }

  /// The quick action type a link names, or nil for other links.
  static func type(of url: URL) -> String? {
    guard url.scheme == "buzz", url.host == "shortcut" else { return nil }
    let type = String(url.path.dropFirst())
    return type.isEmpty ? nil : type
  }
}
