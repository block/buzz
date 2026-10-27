import Flutter
import UIKit

class SceneDelegate: FlutterSceneDelegate {

}

/// Owns scanner visibility without replacing Flutter's normal status-bar policy.
@objc(BuzzFlutterViewController)
class BuzzFlutterViewController: FlutterViewController {
  private var scannerHidesStatusBar = false

  override var prefersStatusBarHidden: Bool {
    scannerHidesStatusBar || super.prefersStatusBarHidden
  }

  override var preferredStatusBarUpdateAnimation: UIStatusBarAnimation {
    UIAccessibility.isReduceMotionEnabled ? .none : .fade
  }

  func setScannerStatusBarHidden(_ hidden: Bool) {
    guard scannerHidesStatusBar != hidden else { return }
    scannerHidesStatusBar = hidden
    UIView.animate(
      withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.23,
      delay: 0,
      options: [.beginFromCurrentState, .curveEaseInOut]
    ) {
      self.setNeedsStatusBarAppearanceUpdate()
    }
  }
}
