import Flutter
import UIKit

/// An unclipped native backing for Flutter overlays. Toasts use a transparent
/// point; moving sheets use a full-size opaque surface so UIKit controls beneath
/// them cannot show through gaps in Flutter's platform-view overlay slices.
final class ToastLayerFactory: NSObject, FlutterPlatformViewFactory {
  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(withFrame frame: CGRect, viewIdentifier viewId: Int64, arguments args: Any?) -> FlutterPlatformView {
    ToastLayerView(frame: frame, arguments: args)
  }
}

private final class ToastLayerView: NSObject, FlutterPlatformView {
  private let surface: UIView

  init(frame: CGRect, arguments: Any?) {
    surface = UIView(frame: frame)
    let values = arguments as? [String: Any]
    if let argb = (values?["color"] as? NSNumber)?.uint32Value {
      surface.backgroundColor = UIColor(
        red: CGFloat((argb >> 16) & 0xff) / 255,
        green: CGFloat((argb >> 8) & 0xff) / 255,
        blue: CGFloat(argb & 0xff) / 255,
        alpha: CGFloat((argb >> 24) & 0xff) / 255
      )
      surface.isOpaque = (argb >> 24) == 0xff
    } else {
      surface.backgroundColor = .clear
      surface.isOpaque = false
    }
    surface.isUserInteractionEnabled = false
    surface.accessibilityElementsHidden = true
    super.init()
  }

  func view() -> UIView { surface }
}
