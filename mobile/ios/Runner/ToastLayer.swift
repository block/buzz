import Flutter
import UIKit

/// An unclipped, transparent composition boundary for Flutter toast content.
/// Flutter paints the toast in the overlay slice above this native view, rather
/// than inheriting the rounded clip of a platform view inside a modal sheet.
final class ToastLayerFactory: NSObject, FlutterPlatformViewFactory {
  func create(withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?) -> FlutterPlatformView {
    ToastLayerView(frame: frame)
  }
}

private final class ToastLayerView: NSObject, FlutterPlatformView {
  private let surface: UIView

  init(frame: CGRect) {
    surface = UIView(frame: frame)
    surface.backgroundColor = .clear
    surface.isOpaque = false
    surface.isUserInteractionEnabled = false
    surface.accessibilityElementsHidden = true
    super.init()
  }

  func view() -> UIView { surface }
}
