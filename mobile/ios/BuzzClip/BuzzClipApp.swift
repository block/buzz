import SwiftUI

/// Buzz App Clip: the not-installed path for a scanned invite QR code.
///
/// The system hands the scanned URL over as a browsing `NSUserActivity`. The
/// clip shows what the user is joining, and "Join" parks the invite in storage
/// the full app reads on first launch (`BuzzPendingInviteStore`) before
/// recommending the full app with `SKOverlay`. Nothing else from the Flutter
/// app is bundled, keeping the clip well under the 15 MB physical-invocation
/// limit.
@main
struct BuzzClipApp: App {
  @StateObject private var model = InviteClipModel(store: BuzzPendingInviteStore())

  var body: some Scene {
    WindowGroup {
      InviteClipView(model: model)
        .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
          model.receive(activity.webpageURL)
        }
        .onAppear(perform: receiveDebugInvocation)
    }
  }

  /// Xcode's `_XCAppClipURL` stands in for a scan while developing. UIKit
  /// turns it into the browsing activity above when Xcode launches the clip;
  /// reading it here too lets `xcrun simctl launch` (with
  /// `SIMCTL_CHILD__XCAppClipURL`) drive the same flow from the command line.
  private func receiveDebugInvocation() {
    #if DEBUG
      guard model.phase == .waitingForInvocation,
        let raw = ProcessInfo.processInfo.environment["_XCAppClipURL"]
      else { return }
      model.receive(URL(string: raw))
    #endif
  }
}
