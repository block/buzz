import AppIntents
import Foundation

// Built into both Runner and BuzzControls. Controls name these actions from
// the extension; the app runs them, handing the matching Home Screen quick
// action to Flutter.

@available(iOS 18.0, *)
struct NewChatControlIntent: AppIntent {
  static var title: LocalizedStringResource = "New chat"
  static var description = IntentDescription("Opens Buzz to a new Solo chat.")
  static var openAppWhenRun = true
  static var isDiscoverable = false

  @MainActor
  func perform() async throws -> some IntentResult {
    openShortcut("new-chat")
    return .result()
  }
}

@available(iOS 18.0, *)
struct MessageBestieControlIntent: AppIntent {
  static var title: LocalizedStringResource = "Message my Bestie"
  static var description = IntentDescription("Opens Buzz to your DM with your Bestie.")
  static var openAppWhenRun = true
  static var isDiscoverable = false

  @MainActor
  func perform() async throws -> some IntentResult {
    openShortcut("message-bestie")
    return .result()
  }
}

@MainActor
private func openShortcut(_ type: String) {
  #if !BUZZ_CONTROLS_EXTENSION
    HomeScreenShortcuts.shared.open(type)
  #endif
}
