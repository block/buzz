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
  private var sending = false
  private var waiters: [UUID: CheckedContinuation<Bool, Never>] = [:]

  func attach(messenger: FlutterBinaryMessenger) {
    let channel = FlutterMethodChannel(name: "buzz/quick_send", binaryMessenger: messenger)
    self.channel = channel
    ready = false
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return result(FlutterMethodNotImplemented) }
      switch call.method {
      case "ready":
        Self.log.notice("dart ready")
        self.ready = true
        let waiting = self.waiters.values
        self.waiters = [:]
        waiting.forEach { $0.resume(returning: true) }
        result(nil)
      case "unready":
        // Home left, for example on sign-out, and took its handler with it.
        self.ready = false
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  /// Waits for Home's `ready`, returning false once [deadline] passes first.
  @MainActor
  private func waitUntilReady(_ deadline: Duration) async -> Bool {
    if ready { return true }
    let id = UUID()
    return await withCheckedContinuation { continuation in
      waiters[id] = continuation
      Task { @MainActor in
        try? await Task.sleep(for: deadline)
        self.waiters.removeValue(forKey: id)?.resume(returning: false)
      }
    }
  }

  /// Sends [text] to the Bestie once [confirm] accepts its name, or throws a
  /// message Siri can read.
  @MainActor
  func send(
    _ text: String,
    deadline: Duration = .seconds(20),
    confirm: (_ bestieName: String?) async throws -> Void
  ) async throws {
    // One at a time, so two quick sends post in the order they were confirmed.
    guard !sending else { throw QuickSendError("Another message is still sending.") }
    sending = true
    defer { sending = false }
    guard await waitUntilReady(deadline) else {
      throw QuickSendError("Open Buzz, then try again.")
    }
    let bestie = try await call("bestie") as? [String: Any]
    guard let pubkey = bestie?["pubkey"] as? String, let scope = bestie?["scope"] as? String else {
      throw QuickSendError("Buzz couldn't find your Bestie.")
    }
    try await confirm(bestie?["name"] as? String)
    let start = ContinuousClock.now
    do {
      _ = try await call("send", arguments: ["text": text, "bestie": pubkey, "scope": scope])
      Self.log.notice("send finished after \(ContinuousClock.now - start, privacy: .public)")
    } catch {
      Self.log.notice("send failed after \(ContinuousClock.now - start, privacy: .public)")
      throw error
    }
  }

  @MainActor
  private func call(_ method: String, arguments: Any? = nil) async throws -> Any? {
    guard ready, let channel else { throw QuickSendError("Open Buzz, then try again.") }
    let reply: Any? = await withCheckedContinuation { continuation in
      channel.invokeMethod(method, arguments: arguments) { continuation.resume(returning: $0) }
    }
    if let error = reply as? FlutterError {
      throw QuickSendError(error.message ?? "Buzz couldn't send that.")
    }
    if reply as? NSObject === FlutterMethodNotImplemented {
      throw QuickSendError("Buzz isn't ready to send yet.")
    }
    return reply
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
  static var description = IntentDescription("Sends a message in your DM with your Bestie without opening Buzz.")
  static var openAppWhenRun = false

  @Parameter(title: "Message", requestValueDialog: "What do you want to say?")
  var message: String

  @MainActor
  func perform() async throws -> some IntentResult & ProvidesDialog {
    let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !text.isEmpty else { throw QuickSendError("Say a message to send.") }
    try await QuickSendBridge.shared.send(text) { name in
      try await requestConfirmation(
        result: .result(dialog: "Send “\(text)” to \(name ?? "your Bestie")?"),
        confirmationActionName: .send
      )
    }
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
