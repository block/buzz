import Flutter
import SwiftUI
import UIKit

final class ComposerToolStripGlassFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger

  init(messenger: FlutterBinaryMessenger) { self.messenger = messenger }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(
    withFrame frame: CGRect,
    viewIdentifier viewId: Int64,
    arguments args: Any?
  ) -> FlutterPlatformView {
    ComposerToolStripGlass(frame: frame, id: viewId, messenger: messenger, arguments: args)
  }
}

private final class ComposerToolStripGlass: NSObject, FlutterPlatformView {
  private let container = UIView()
  private let channel: FlutterMethodChannel
  private var hostingController: UIViewController?
  private var updateGeometry: ((CGSize, String, Bool) -> Void)?

  init(frame: CGRect, id: Int64, messenger: FlutterBinaryMessenger, arguments: Any?) {
    channel = FlutterMethodChannel(name: "buzz/composer_tool_strip_glass/\(id)", binaryMessenger: messenger)
    super.init()
    container.frame = frame
    container.isOpaque = false
    container.backgroundColor = .clear
    container.isUserInteractionEnabled = false
    let brightness = (arguments as? [String: Any])?["brightness"] as? String
    container.overrideUserInterfaceStyle = brightness == "dark" ? .dark : .light
    if #available(iOS 26.0, *) {
      let model = ComposerGlassModel()
      let host = UIHostingController(rootView: ComposerMatchedGlass(model: model))
      host.view.isOpaque = false
      host.view.backgroundColor = .clear
      host.view.isUserInteractionEnabled = false
      host.view.frame = container.bounds
      host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
      container.addSubview(host.view)
      hostingController = host
      updateGeometry = { size, phase, reduced in
        let next = ComposerGlassGeometry(size: size, phase: phase)
        guard model.geometry != next else { return }
        let animate = model.geometry != nil && !reduced && !UIAccessibility.isReduceMotionEnabled
        withAnimation(animate ? .timingCurve(0.645, 0.045, 0.355, 1, duration: 0.18) : nil) {
          model.geometry = next
        }
      }
    } else {
      let surface = UIVisualEffectView(effect: UIBlurEffect(style: .systemMaterial))
      surface.autoresizingMask = [.flexibleTopMargin]
      surface.layer.cornerRadius = 24
      surface.layer.cornerCurve = .continuous
      surface.clipsToBounds = true
      container.addSubview(surface)
      updateGeometry = { [weak container] size, _, _ in
        guard let container else { return }
        surface.frame = CGRect(x: 0, y: container.bounds.height - size.height, width: size.width, height: size.height)
      }
    }
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "geometry",
        let args = call.arguments as? [String: Any],
        let width = args["width"] as? Double,
        let height = args["height"] as? Double,
        width.isFinite, height.isFinite, width >= 0, height >= 0,
        let phase = args["phase"] as? String
      else {
        result(FlutterMethodNotImplemented)
        return
      }
      self?.updateGeometry?(CGSize(width: width, height: height), phase, args["reducedMotion"] as? Bool ?? false)
      result(nil)
    }
  }

  func view() -> UIView { container }

  deinit { channel.setMethodCallHandler(nil) }
}

private struct ComposerGlassGeometry: Equatable {
  let size: CGSize
  let phase: String
}

@available(iOS 26.0, *)
private final class ComposerGlassModel: ObservableObject {
  @Published var geometry: ComposerGlassGeometry?
}

@available(iOS 26.0, *)
private struct ComposerMatchedGlass: View {
  @ObservedObject var model: ComposerGlassModel
  @Namespace private var glassNamespace

  var body: some View {
    GlassEffectContainer {
      ZStack(alignment: .bottomLeading) {
        if let geometry = model.geometry {
          Color.clear
            .frame(width: geometry.size.width, height: geometry.size.height)
            .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 24))
            .glassEffectID("composer-tools", in: glassNamespace)
            .glassEffectTransition(.matchedGeometry)
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
    }
    .ignoresSafeArea()
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }
}
