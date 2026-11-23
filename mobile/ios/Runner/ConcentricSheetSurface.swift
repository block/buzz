import Flutter
import UIKit

final class ConcentricSheetSurfaceFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger

  init(messenger: FlutterBinaryMessenger) {
    self.messenger = messenger
    super.init()
  }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    ConcentricSheetSurfacePlatformView(
      frame: frame,
      viewIdentifier: viewId,
      messenger: messenger,
      arguments: args
    )
  }
}

final class ConcentricSheetSurfacePlatformView: NSObject, FlutterPlatformView {
  private let rootView: UIView
  private let surfaceView: UIView
  private let independentCorners: Bool
  private let channel: FlutterMethodChannel
  private let usesGlass: Bool
  private let animateGeometry: Bool
  private let topRadius: Double?

  init(
    frame: CGRect,
    viewIdentifier viewId: Int64,
    messenger: FlutterBinaryMessenger,
    arguments args: Any?
  ) {
    let arguments = args as? [String: Any]
    let colorValue = (arguments?["color"] as? NSNumber)?.uint32Value ?? 0xFFFF_FFFF
    let minimumRadius = (arguments?["minimumRadius"] as? NSNumber)?.doubleValue ?? 24
    topRadius = (arguments?["topRadius"] as? NSNumber)?.doubleValue
    independentCorners = (arguments?["independentCorners"] as? NSNumber)?.boolValue == true
    usesGlass = (arguments?["usesGlass"] as? NSNumber)?.boolValue == true
    animateGeometry = arguments?["animateGeometry"] as? Bool == true

    rootView = ConcentricSurfaceRootView(frame: frame)
    rootView.isUserInteractionEnabled = false
    rootView.isOpaque = false
    rootView.backgroundColor = .clear

    if #available(iOS 26.0, *), usesGlass {
      let effect = UIGlassEffect(style: .regular)
      effect.isInteractive = false
      surfaceView = UIVisualEffectView(effect: effect)
    } else {
      surfaceView = UIView()
    }
    surfaceView.frame = rootView.bounds
    surfaceView.isUserInteractionEnabled = false
    surfaceView.isOpaque = !usesGlass
    surfaceView.clipsToBounds = true
    surfaceView.layer.cornerCurve = .continuous
    surfaceView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    rootView.addSubview(surfaceView)

    channel = FlutterMethodChannel(
      name: "buzz/concentric_sheet_surface/\(viewId)",
      binaryMessenger: messenger
    )

    super.init()

    updateColors(
      colorValue: colorValue,
      glassTintColorValue: (arguments?["glassTintColor"] as? NSNumber)?.uint32Value
    )
    applyInterfaceStyle(from: arguments?["brightness"])
    applyCornerConfiguration(minimumRadius: minimumRadius, capsule: arguments?["capsule"] as? Bool == true)

    (rootView as? ConcentricSurfaceRootView)?.onLayout = { [weak self] in
      guard let self else { return }
      self.channel.invokeMethod("cornerRadiiChanged", arguments: self.cornerRadii())
    }

    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else {
        result(nil)
        return
      }
      switch call.method {
      case "getCornerRadii":
        self.rootView.layoutIfNeeded()
        result(self.cornerRadii())
      case "updateColors":
        guard
          let arguments = call.arguments as? [String: Any],
          let colorValue = arguments["color"] as? NSNumber
        else {
          result(
            FlutterError(
              code: "invalid_arguments",
              message: "Expected a surface color.",
              details: nil
            )
          )
          return
        }

        self.updateColors(
          colorValue: colorValue.uint32Value,
          glassTintColorValue: (arguments["glassTintColor"] as? NSNumber)?.uint32Value
        )
        result(nil)
      case "updateGeometry":
        guard
          let arguments = call.arguments as? [String: Any],
          let minimumRadius = arguments["minimumRadius"] as? NSNumber
        else {
          result(
            FlutterError(
              code: "invalid_arguments",
              message: "Expected a minimum corner radius.",
              details: nil
            )
          )
          return
        }
        self.applyInterfaceStyle(from: arguments["brightness"])
        let update = {
          self.applyCornerConfiguration(
            minimumRadius: minimumRadius.doubleValue,
            capsule: arguments["capsule"] as? Bool == true
          )
          self.surfaceView.layoutIfNeeded()
        }
        if self.animateGeometry && !UIAccessibility.isReduceMotionEnabled {
          UIView.animate(withDuration: 0.18, delay: 0, options: [.beginFromCurrentState, .allowUserInteraction, .curveEaseInOut], animations: update)
        } else { update() }
        result(nil)
      default:
        result(FlutterMethodNotImplemented)
      }
    }
  }

  deinit {
    channel.setMethodCallHandler(nil)
  }

  func view() -> UIView {
    rootView
  }

  private func updateColors(
    colorValue: UInt32,
    glassTintColorValue: UInt32?
  ) {
    let surfaceColor = Self.color(from: colorValue)
    if let glassView = surfaceView as? UIVisualEffectView {
      if #available(iOS 26.0, *), let effect = glassView.effect as? UIGlassEffect {
        effect.tintColor = glassTintColorValue.map { Self.color(from: $0) }
        glassView.effect = effect
      }
      glassView.backgroundColor = .clear
      glassView.contentView.backgroundColor = surfaceColor.withAlphaComponent(min(surfaceColor.cgColor.alpha, 0.12))
    } else {
      surfaceView.backgroundColor = surfaceColor
    }
    rootView.isOpaque = false
    rootView.backgroundColor = .clear
  }

  private func applyCornerConfiguration(minimumRadius: CGFloat, capsule: Bool) {
    if #available(iOS 26.0, *) {
      if capsule {
        surfaceView.cornerConfiguration = .capsule()
        return
      }
      let radius = UICornerRadius.containerConcentric(minimum: minimumRadius)
      if independentCorners {
        surfaceView.cornerConfiguration = .corners(
          topLeftRadius: topRadius.map { .fixed($0) } ?? radius,
          topRightRadius: topRadius.map { .fixed($0) } ?? radius,
          bottomLeftRadius: radius,
          bottomRightRadius: radius
        )
        return
      }
      surfaceView.cornerConfiguration =
        topRadius.map {
          .uniformBottomRadius(
            radius,
            topLeftRadius: .fixed($0),
            topRightRadius: .fixed($0)
          )
        } ?? .uniformCorners(radius: radius)
    } else {
      surfaceView.layer.cornerRadius = minimumRadius
    }
  }

  private func cornerRadii() -> [String: CGFloat] {
    if #available(iOS 26.0, *) {
      return [
        "topLeft": surfaceView.effectiveRadius(corner: .topLeft),
        "topRight": surfaceView.effectiveRadius(corner: .topRight),
        "bottomLeft": surfaceView.effectiveRadius(corner: .bottomLeft),
        "bottomRight": surfaceView.effectiveRadius(corner: .bottomRight),
      ]
    }
    let radius = surfaceView.layer.cornerRadius
    return [
      "topLeft": radius,
      "topRight": radius,
      "bottomLeft": radius,
      "bottomRight": radius,
    ]
  }

  private func applyInterfaceStyle(from value: Any?) {
    switch value as? String {
    case "dark":
      rootView.overrideUserInterfaceStyle = .dark
    case "light":
      rootView.overrideUserInterfaceStyle = .light
    default:
      rootView.overrideUserInterfaceStyle = .unspecified
    }
  }

  private static func color(from value: UInt32) -> UIColor {
    let alpha = CGFloat((value >> 24) & 0xFF) / 255
    let red = CGFloat((value >> 16) & 0xFF) / 255
    let green = CGFloat((value >> 8) & 0xFF) / 255
    let blue = CGFloat(value & 0xFF) / 255
    return UIColor(red: red, green: green, blue: blue, alpha: alpha)
  }
}

/// Reading effective radii during layout lets UIKit invalidate the geometry
/// when the enclosing display or presentation changes.
private final class ConcentricSurfaceRootView: UIView {
  var onLayout: (() -> Void)?

  override func layoutSubviews() {
    super.layoutSubviews()
    onLayout?()
  }
}
