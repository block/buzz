import AppIntents
import Flutter
import os.log

/// Spike: reports whether Dart answers when an App Intent wakes Buzz.
///
/// The channel attaches when the Flutter engine initializes. If iOS runs the
/// intent without connecting a scene and the engine never starts, `ping`
/// reports that instead of waiting forever.
final class QuickSendBridge {
  static let shared = QuickSendBridge()
  static let log = Logger(subsystem: "xyz.block.buzz", category: "QuickSend")

  private var channel: FlutterMethodChannel?
  private var waiters: [CheckedContinuation<Void, Never>] = []

  func attach(messenger: FlutterBinaryMessenger) {
    Self.log.notice("engine attached")
    channel = FlutterMethodChannel(name: "buzz/quick_send", binaryMessenger: messenger)
    let ready = waiters
    waiters = []
    ready.forEach { $0.resume() }
  }

  @MainActor
  func ping(timeout: Duration) async -> String {
    let start = ContinuousClock.now
    if channel == nil {
      Self.log.notice("no engine yet; waiting")
      await withTaskGroup(of: Void.self) { group in
        group.addTask { @MainActor in
          await withCheckedContinuation { self.waiters.append($0) }
        }
        group.addTask { try? await Task.sleep(for: timeout) }
        await group.next()
        group.cancelAll()
      }
    }
    guard let channel else {
      Self.log.error("engine never attached")
      return "no engine after \(timeout)"
    }
    return await withCheckedContinuation { continuation in
      var finished = false
      let finish: (String) -> Void = { value in
        guard !finished else { return }
        finished = true
        continuation.resume(returning: value)
      }
      channel.invokeMethod("ping", arguments: nil) { reply in
        let elapsed = ContinuousClock.now - start
        Self.log.notice("dart replied: \(String(describing: reply), privacy: .public) after \(elapsed, privacy: .public)")
        finish("\(reply ?? "nil") after \(elapsed)")
      }
      DispatchQueue.main.asyncAfter(deadline: .now() + .seconds(Int(timeout.components.seconds))) {
        finish("dart silent after \(timeout)")
      }
    }
  }
}

@available(iOS 16.0, *)
struct QuickSendProbeIntent: AppIntent {
  static var title: LocalizedStringResource = "Buzz background probe"
  static var openAppWhenRun = false

  @MainActor
  func perform() async throws -> some IntentResult & ProvidesDialog {
    let state = UIApplication.shared.applicationState
    QuickSendBridge.log.notice("probe perform, appState=\(state.rawValue, privacy: .public)")
    let result = await QuickSendBridge.shared.ping(timeout: .seconds(8))
    return .result(dialog: "Buzz: \(result) (app state \(state.rawValue))")
  }
}

@available(iOS 16.0, *)
struct BuzzAppShortcuts: AppShortcutsProvider {
  static var appShortcuts: [AppShortcut] {
    AppShortcut(
      intent: QuickSendProbeIntent(),
      phrases: ["Run the \(.applicationName) background probe"],
      shortTitle: "Background probe",
      systemImageName: "waveform"
    )
  }
}
