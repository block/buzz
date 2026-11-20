import Foundation

/// Links that run a Home Screen quick action, for example
/// `buzz://shortcut/new-chat`.
enum ShortcutLink {
  /// The quick action type a link names, or nil for other links.
  static func type(of url: URL) -> String? {
    guard url.scheme == "buzz", url.host == "shortcut" else { return nil }
    let type = String(url.path.dropFirst())
    return type.isEmpty ? nil : type
  }
}
