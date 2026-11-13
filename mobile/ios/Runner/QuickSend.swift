import AppIntents
import Flutter
import os.log

/// Hands quick sends from Siri and Shortcuts to Dart.
///
/// A background launch starts the app's Flutter engine and builds Home, which
/// says `ready` once it can take sends. A send that arrives first waits for
/// that, up to a deadline.
final class QuickSendBridge {
  static let shared = QuickSendBridge()
  static let log = Logger(subsystem: "xyz.block.buzz", category: "QuickSend")

  private var channel: FlutterMethodChannel?
  private var ready = false
  private var waiters: [CheckedContinuation<Void, Never>] = []

  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "buzz/quick_send", binaryMessenger: messenger)
    self.channel = channel
    ready = false
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "ready", let self else {
        result(FlutterMethodNotImplemented)
        return
      }
      Self.log.notice("dart ready")
      self.ready = true
      let waiting = self.waiters
      self.waiters = []
      waiting.forEach { $0.resume() }
      result(nil)
    }
  }

  /// Sends [text] to the Bestie, or throws a message Siri can read.
  @MainActor
  func send(_ text: String, deadline: Duration = .seconds(20)) async throws {
    let start = ContinuousClock.now
    if !ready {
      await withTaskGroup(of: Void.self) { group in
        group.addTask { @MainActor in
          await withCheckedContinuation { self.waiters.append($0) }
        }
        group.addTask { try? await Task.sleep(for: deadline) }
        await group.next()
        group.cancelAll()
      }
    }
    // A timed-out waiter stays queued and is resumed by a later `ready`.
    guard ready, let channel else { throw QuickSendError("Buzz didn't start in time.") }
    let failure: String? = await withCheckedContinuation { continuation in
      channel.invokeMethod("send", arguments: text) { reply in
        if let error = reply as? FlutterError {
          continuation.resume(returning: error.message ?? "Buzz couldn't send that.")
        } else if reply as? NSObject === FlutterMethodNotImplemented {
          continuation.resume(returning: "Buzz isn't ready to send yet.")
        } else {
          continuation.resume(returning: nil)
        }
      }
    }
    Self.log.notice("send finished after \(ContinuousClock.now - start, privacy: .public), failed=\(failure != nil, privacy: .public)")
    if let failure { throw QuickSendError(failure) }
  }
}

struct QuickSendError: Error, CustomLocalizedStringResourceConvertible {
  let message: String
  init(_ message: String) { self.message = message }
  var localizedStringResource: LocalizedStringResource { "\(message)" }
}

@available(iOS 16.0, *)
struct MessageBestieIntent: AppIntent {
  static var title: LocalizedStringResource = "Message my Bestie"
  static var description = IntentDescription("Starts a new Solo chat with your Bestie without opening Buzz.")
  static var openAppWhenRun = false

  @Parameter(title: "Message", requestValueDialog: "What do you want to say?")
  var message: String

  @MainActor
  func perform() async throws -> some IntentResult & ProvidesDialog {
    let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty else { throw QuickSendError("Say a message to send.") }
    try await requestConfirmation(
      result: .result(dialog: "Send “\(text)” to your Bestie?"),
      confirmationActionName: .send
    )
    try await QuickSendBridge.shared.send(text)
    return .result(dialog: "Sent.")
  }
}

@available(iOS 16.0, *)
struct BuzzAppShortcuts: AppShortcutsProvider {
  static var appShortcuts: [AppShortcut] {
    AppShortcut(
      intent: MessageBestieIntent(),
      phrases: [
        "Message my Bestie in \(.applicationName)",
        "Send a message to my Bestie in \(.applicationName)",
      ],
      shortTitle: "Message my Bestie",
      systemImageName: "bubble.left.and.text.bubble.right"
    )
  }
}
