import Flutter
import UIKit
import CoreText

/// Profile draft presentation; persistence and the existing avatar editor stay in Dart.
final class NativeProfileSetupCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private var navigation: UINavigationController?
  private var pending: FlutterResult?
  private var session: Int?
  private var avatarController: FlutterViewController?
  private var avatarEngine: FlutterEngine?
  private var avatarChannel: FlutterMethodChannel?
  private var transitionCover: UIView?
  private var transitionAvatar: UIView?
  private var disabledSheetPans: [UIGestureRecognizer] = []
  private var previousInteractivePopEnabled: Bool?
  private var previousDetent: UISheetPresentationController.Detent.Identifier?
  private var editorAvatarRect: CGRect?
  private var avatarDraft: [String: Any]?
  private let scheduleEditorWatchdog: (@escaping () -> Void) -> Void
  private let startEditorEngine: (FlutterEngine) -> Bool
  private let configureEditorEngine: (FlutterEngine) -> Void


  init(messenger: FlutterBinaryMessenger, scheduleEditorWatchdog: @escaping (@escaping () -> Void) -> Void = {
         DispatchQueue.main.asyncAfter(deadline: .now() + 2, execute: $0)
       }, startEditorEngine: @escaping (FlutterEngine) -> Bool = {
         $0.run(withEntrypoint: "profileAvatarEditorMain", libraryURI: "package:buzz/features/profile/profile_avatar_sheet_app.dart")
       }, configureEditorEngine: @escaping (FlutterEngine) -> Void) {
    self.scheduleEditorWatchdog = scheduleEditorWatchdog
    self.startEditorEngine = startEditorEngine
    self.configureEditorEngine = configureEditorEngine
    channel = FlutterMethodChannel(name: "buzz/profile_setup", binaryMessenger: messenger)
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return }
      if call.method == "dismiss" {
        guard let args = call.arguments as? [String: Any], args["session"] as? Int == self.session else { result(nil); return }
        self.finish("cancel"); result(nil); return
      }
      guard call.method == "present", let args = call.arguments as? [String: Any],
            let session = args["session"] as? Int else { result(FlutterMethodNotImplemented); return }
      guard self.pending == nil else { result(FlutterError(code: "busy", message: nil, details: nil)); return }
      let root = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive || $0.activationState == .foregroundInactive }.flatMap(\.windows)
        .first(where: \.isKeyWindow)?.rootViewController
      guard var presenter = root else { result(FlutterError(code: "unavailable", message: nil, details: nil)); return }
      while let next = presenter.presentedViewController { presenter = next }
      let controller = NativeProfileSetupController(args: args)
      controller.finish = { [weak self] action in self?.finish(action) }
      controller.openAvatar = { [weak self, weak controller] in
        guard let self, let controller else { return }; self.openAvatar(from: controller, args: args, session: session)
      }
      controller.action = { [weak self] action, payload, completion in
        guard let self, self.session == session else { return }
        self.channel.invokeMethod(action, arguments: payload.merging(["session": session]) { _, new in new }) { value in
          guard self.session == session else { return }
          completion(value as? [String: Any] ?? [:])
        }
      }
      let nav = UINavigationController(rootViewController: controller)
      nav.modalPresentationStyle = .pageSheet
      nav.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
      self.pending = result; self.session = session; self.navigation = nav
      controller.loadViewIfNeeded()
      nav.sheetPresentationController?.detents = [.custom(identifier: .init("profile")) { _ in 430 }, .large()]
      nav.sheetPresentationController?.selectedDetentIdentifier = args["expanded"] as? Bool == true ? .large : .init("profile")
      nav.sheetPresentationController?.prefersScrollingExpandsWhenScrolledToEdge = false
      presenter.present(nav, animated: !UIAccessibility.isReduceMotionEnabled)
      nav.presentationController?.delegate = self
    }
  }
  private func openAvatar(from controller: NativeProfileSetupController, args: [String: Any], session: Int) {
    guard self.session == session, avatarEngine == nil, let navigation else { return }
    previousInteractivePopEnabled = navigation.interactivePopGestureRecognizer?.isEnabled
    previousDetent = navigation.sheetPresentationController?.selectedDetentIdentifier
    let sourceAvatar = controller.avatarSnapshot(in: navigation.view)
    controller.setAvatarHidden(true)
    let cover = navigation.view.snapshotView(afterScreenUpdates: true)
    controller.setAvatarHidden(false)
    controller.view.endEditing(true)
    navigation.isModalInPresentation = true
    var sheetView: UIView? = navigation.view
    while let current = sheetView {
      for gesture in current.gestureRecognizers ?? [] where gesture is UIPanGestureRecognizer && gesture.isEnabled {
        gesture.isEnabled = false; disabledSheetPans.append(gesture)
      }
      if current === navigation.presentationController?.containerView { break }
      sheetView = current.superview
    }
    navigation.sheetPresentationController?.animateChanges {
      navigation.sheetPresentationController?.detents = [.large()]
      navigation.sheetPresentationController?.selectedDetentIdentifier = .large
    }
    let engine = FlutterEngine(name: "profile-avatar-\(session)", project: nil, allowHeadlessExecution: false)
    avatarEngine = engine
    guard startEditorEngine(engine) else {
      restoreProfileSheet(controller); return
    }
    let editorChannel = FlutterMethodChannel(name: "buzz/profile_avatar_sheet", binaryMessenger: engine.binaryMessenger)
    avatarChannel = editorChannel
    var ready = false
    var finishing = false
    editorChannel.setMethodCallHandler { [weak self, weak controller, weak engine] call, result in
      guard let self, let engine, self.session == session, self.avatarEngine === engine, let controller else { result(nil); return }
      if call.method == "ready", let rect = call.arguments as? [String: NSNumber], let editor = self.avatarController {
        ready = true
        let target = CGRect(x: rect["x"]?.doubleValue ?? 0, y: rect["y"]?.doubleValue ?? 0, width: rect["width"]?.doubleValue ?? 220, height: rect["height"]?.doubleValue ?? 220)
        self.editorAvatarRect = target
        let destination = editor.view.convert(target, to: navigation.view)
        self.animateAvatarMotion {
          self.transitionCover?.alpha = 0
          self.transitionAvatar?.frame = destination
        } completion: {
          result(nil)
          UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.08) {
            self.transitionAvatar?.alpha = 0
          } completion: { _ in self.clearTransition() }
        }
        return
      }
      if call.method == "initial" { result(["dark": args["dark"] ?? false, "draft": self.avatarDraft ?? [:]]); return }
      guard call.method == "finish", !finishing else { result(FlutterMethodNotImplemented); return }
      finishing = true
      let draft = call.arguments as? [String: Any]
      func returnToProfile() {
        self.clearTransition()
        let editor = self.avatarController
        let rect = self.editorAvatarRect
        let avatarSnapshot = rect.flatMap { editor?.view.resizableSnapshotView(from: $0, afterScreenUpdates: false, withCapInsets: .zero) }
        let source = rect.flatMap { editor?.view.convert($0, to: navigation.view) }
        let cover = navigation.view.snapshotView(afterScreenUpdates: false)
        if let cover, let source {
          // Cut the moving avatar out of the outgoing page snapshot.
          let mask = CAShapeLayer()
          let path = UIBezierPath(rect: navigation.view.bounds)
          path.append(UIBezierPath(rect: source))
          mask.path = path.cgPath; mask.fillRule = .evenOdd
          cover.layer.mask = mask
        }
        controller.setAvatarHidden(true)
        navigation.setNavigationBarHidden(false, animated: false)
        navigation.popViewController(animated: false)
        navigation.view.layoutIfNeeded()
        let destination = controller.avatarFrame(in: navigation.view)
        if let cover { cover.frame = navigation.view.bounds; navigation.view.addSubview(cover); self.transitionCover = cover }
        if let avatarSnapshot, let source {
          avatarSnapshot.frame = source; navigation.view.addSubview(avatarSnapshot); self.transitionAvatar = avatarSnapshot
        }
        // Start the sheet resize alongside the shared-avatar return.
        navigation.sheetPresentationController?.animateChanges {
          self.restoreProfileDetents()
        }
        self.animateAvatarMotion(returning: true) {
          self.transitionCover?.alpha = 0
          self.transitionAvatar?.frame = destination
        } completion: { [weak self, weak controller] in
          guard let self, self.session == session else { return }
          self.restoreProfileSheet(controller)
        }
        result(nil)
      }
      if let draft {
        self.avatarDraft = draft
        self.channel.invokeMethod("avatarDraft", arguments: ["session": session, "draft": draft]) { value in
          guard self.session == session else { result(nil); return }
          controller.updateAvatar(value as? [String: Any] ?? [:]); returnToProfile()
        }
      } else { returnToProfile() }
    }
    GeneratedPluginRegistrant.register(with: engine)
    configureEditorEngine(engine)
    let messenger = engine.binaryMessenger
    engine.registrar(forPlugin: "BuzzNavigationGlassButton")?.register(NavigationGlassButtonFactory(messenger: messenger), withId: "buzz/navigation_glass")
    engine.registrar(forPlugin: "BuzzNativeSegmentedControl")?.register(NativeSegmentedControlFactory(messenger: messenger), withId: "buzz/native_segmented_control")
    engine.registrar(forPlugin: "BuzzNativeSkinToneControl")?.register(NativeSkinToneControlFactory(messenger: messenger), withId: "buzz/native_skin_tone_control")
    let editor = FlutterViewController(engine: engine, nibName: nil, bundle: nil)
    avatarController = editor
    engine.registrar(forPlugin: "BuzzIosNavigationBar")?.register(IosNavigationBarFactory(messenger: messenger, parent: editor), withId: "buzz/ios_navigation_bar")
    editor.view.backgroundColor = controller.view.backgroundColor
    navigation.setNavigationBarHidden(true, animated: false)
    navigation.pushViewController(editor, animated: false)
    if let cover { cover.frame = navigation.view.bounds; navigation.view.addSubview(cover); transitionCover = cover }
    if let (snapshot, frame) = sourceAvatar { snapshot.frame = frame; navigation.view.addSubview(snapshot); transitionAvatar = snapshot }
    // Recover the whole sheet when Dart never delivers the editor's first frame.
    scheduleEditorWatchdog { [weak self, weak engine, weak controller] in
      guard let self, let engine, self.session == session,
            self.avatarEngine === engine, !ready, !finishing else { return }
      navigation.setNavigationBarHidden(false, animated: false)
      navigation.popViewController(animated: false)
      self.restoreProfileSheet(controller)
    }
    navigation.interactivePopGestureRecognizer?.isEnabled = false

  }

  // Profile editing uses 200ms easeOutCubic on entrance. Its reversed
  // easeInCubic value curve is mirrored below for the visible return motion.
  private func animateAvatarMotion(returning: Bool = false, _ animations: @escaping () -> Void, completion: @escaping () -> Void) {
    if UIAccessibility.isReduceMotionEnabled { animations(); completion(); return }
    let timing = returning
      ? UICubicTimingParameters(controlPoint1: CGPoint(x: 0.325, y: 0.81), controlPoint2: CGPoint(x: 0.45, y: 0.945))
      : UICubicTimingParameters(controlPoint1: CGPoint(x: 0.215, y: 0.61), controlPoint2: CGPoint(x: 0.355, y: 1))
    let animator = UIViewPropertyAnimator(duration: 0.2, timingParameters: timing)
    animator.addAnimations(animations)
    animator.addCompletion { _ in completion() }
    animator.startAnimation()
  }

  private func restoreProfileDetents() {
    navigation?.sheetPresentationController?.detents = [.custom(identifier: .init("profile")) { _ in 430 }, .large()]
    navigation?.sheetPresentationController?.selectedDetentIdentifier = previousDetent ?? .init("profile")
  }

  /// Both failed startup and normal editor return must leave a retryable sheet.
  private func restoreProfileSheet(_ controller: NativeProfileSetupController?) {
    restoreProfileDetents()
    previousDetent = nil
    clearTransition()
    avatarChannel?.setMethodCallHandler(nil); avatarChannel = nil
    avatarEngine?.destroyContext(); avatarEngine = nil
    avatarController = nil; editorAvatarRect = nil
    navigation?.isModalInPresentation = false
    if let enabled = previousInteractivePopEnabled { navigation?.interactivePopGestureRecognizer?.isEnabled = enabled }
    previousInteractivePopEnabled = nil
    disabledSheetPans.forEach { $0.isEnabled = true }; disabledSheetPans.removeAll()
    controller?.setAvatarHidden(false)
    controller?.avatarReady()
  }

  private func clearTransition() {
    transitionCover?.removeFromSuperview(); transitionCover = nil
    transitionAvatar?.removeFromSuperview(); transitionAvatar = nil
  }
  private func finish(_ action: String) {
    navigation?.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in self?.complete(action) }
  }
  private func complete(_ action: String) {
    clearTransition()
    disabledSheetPans.forEach { $0.isEnabled = true }; disabledSheetPans.removeAll()
    let result = pending; pending = nil; navigation = nil; session = nil
    avatarChannel?.setMethodCallHandler(nil); avatarChannel = nil; avatarEngine?.destroyContext(); avatarEngine = nil; avatarController = nil; avatarDraft = nil
    result?(action)
  }
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { complete("skip") }
}

private final class NativeProfileSetupController: UIViewController, UITextFieldDelegate {
  var finish: ((String) -> Void)?
  var action: ((String, [String: Any], @escaping ([String: Any]) -> Void) -> Void)?
  var openAvatar: (() -> Void)?
  private let args: [String: Any]
  private let name = UITextField()
  private let avatar = UIButton(type: .system)
  private let status = UILabel()
  private var controls: [UIButton] = []
  private var submit: UIButton!
  private var draftName: String
  init(args: [String: Any]) { self.args = args; draftName = args["name"] as? String ?? ""; super.init(nibName: nil, bundle: nil) }
  required init?(coder: NSCoder) { nil }
  override func viewDidLoad() {
    super.viewDidLoad()
    title = "Set up your profile"
    view.backgroundColor = color("pageBackgroundArgb", fallback: .systemBackground)
    let argb = (args["primaryArgb"] as? NSNumber)?.uint32Value ?? 0xFF007AFF
    view.tintColor = UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255, alpha: 1)
    navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in self?.finish?("skip") })
    let stack = UIStackView(); stack.axis = .vertical; stack.spacing = 16; stack.translatesAutoresizingMaskIntoConstraints = false
    let scroll = UIScrollView(); scroll.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(scroll); scroll.addSubview(stack)
    NSLayoutConstraint.activate([
      scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor), scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
      scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor), scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      stack.heightAnchor.constraint(greaterThanOrEqualTo: scroll.frameLayoutGuide.heightAnchor, constant: -40),
      stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 20), stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -20),
      stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 20), stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -20)
    ])
    let avatarRow = UIView(); avatarRow.heightAnchor.constraint(equalToConstant: 112).isActive = true
    avatar.translatesAutoresizingMaskIntoConstraints = false
    avatar.accessibilityLabel = "Add profile photo"
    updateAvatar(args)
    avatar.addAction(UIAction { [weak self] _ in
      guard let self else { return }; self.avatar.isEnabled = false; self.openAvatar?()
    }, for: .touchUpInside)
    avatarRow.addSubview(avatar)
    NSLayoutConstraint.activate([avatar.centerXAnchor.constraint(equalTo: avatarRow.centerXAnchor), avatar.topAnchor.constraint(equalTo: avatarRow.topAnchor), avatar.widthAnchor.constraint(equalToConstant: 112), avatar.heightAnchor.constraint(equalToConstant: 112)])
    stack.addArrangedSubview(avatarRow); controls.append(avatar)
    name.placeholder = "Display name"; name.accessibilityLabel = "Display name"; name.text = draftName
    name.font = .preferredFont(forTextStyle: .body); name.adjustsFontForContentSizeCategory = true
    name.backgroundColor = color("containerBackgroundArgb", fallback: .secondarySystemBackground); name.layer.cornerRadius = CGFloat((args["containerCornerRadius"] as? NSNumber)?.doubleValue ?? 22)
    name.clearButtonMode = .whileEditing; name.autocapitalizationType = .words; name.returnKeyType = .done
    name.leftView = UIView(frame: CGRect(x: 0, y: 0, width: 20, height: 1)); name.leftViewMode = .always
    name.heightAnchor.constraint(greaterThanOrEqualToConstant: 56).isActive = true
    name.delegate = self; name.addTarget(self, action: #selector(nameChanged), for: .editingChanged)
    stack.addArrangedSubview(name)
    status.numberOfLines = 0; status.textColor = .systemRed; status.font = .preferredFont(forTextStyle: .footnote); status.isHidden = true; stack.addArrangedSubview(status)
    let spacer = UIView(); spacer.setContentHuggingPriority(.init(1), for: .vertical)
    spacer.heightAnchor.constraint(greaterThanOrEqualToConstant: 12).isActive = true; stack.addArrangedSubview(spacer)
    submit = UIButton(configuration: onboardingButtonConfiguration("Continue", primary: true), primaryAction: UIAction { [weak self] _ in self?.save() })
    let skip = UIButton(configuration: onboardingButtonConfiguration("Skip", primary: false), primaryAction: UIAction { [weak self] _ in self?.finish?("skip") })
    for button in [submit!, skip] { button.heightAnchor.constraint(greaterThanOrEqualToConstant: 52).isActive = true; stack.addArrangedSubview(button); controls.append(button) }
    updateName()
  }
  @objc private func nameChanged() { draftName = name.text ?? ""; updateName() }
  func textFieldShouldReturn(_ textField: UITextField) -> Bool { textField.resignFirstResponder(); return true }
  func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
    guard let current = textField.text, let range = Range(range, in: current) else { return false }
    return current.replacingCharacters(in: range, with: string).count <= 100
  }
  func setAvatarHidden(_ hidden: Bool) { avatar.isHidden = hidden }
  func avatarFrame(in container: UIView) -> CGRect { avatar.convert(avatar.bounds, to: container) }
  func avatarSnapshot(in container: UIView) -> (UIView, CGRect)? {
    guard let snapshot = avatar.snapshotView(afterScreenUpdates: false) else { return nil }
    return (snapshot, avatar.convert(avatar.bounds, to: container))
  }
  func avatarReady() { avatar.isEnabled = true }
  func updateAvatar(_ args: [String: Any]) {
    avatar.layer.sublayers?.filter { $0 is CAShapeLayer }.forEach { $0.removeFromSuperlayer() }
    avatar.setImage(nil, for: .normal); avatar.setTitle(nil, for: .normal); avatar.backgroundColor = .clear
    if let bytes = args["avatar"] as? FlutterStandardTypedData, let image = UIImage(data: bytes.data) {
      avatar.setImage(image.withRenderingMode(.alwaysOriginal), for: .normal); avatar.imageView?.contentMode = .scaleAspectFill
      avatar.clipsToBounds = true; avatar.layer.cornerRadius = 56
    } else if let emoji = args["emoji"] as? String {
      avatar.setTitle(emoji, for: .normal); avatar.titleLabel?.font = .systemFont(ofSize: 56)
      let color = (args["emojiColor"] as? NSNumber)?.uint32Value ?? 0xFFFFFFFF
      avatar.backgroundColor = UIColor(red: CGFloat((color >> 16) & 255) / 255, green: CGFloat((color >> 8) & 255) / 255, blue: CGFloat(color & 255) / 255, alpha: 1); avatar.layer.cornerRadius = 56
    } else {
      avatar.setImage(tablerPlusImage(), for: .normal)
      let ring = CAShapeLayer(); ring.path = UIBezierPath(ovalIn: CGRect(x: 2, y: 2, width: 108, height: 108)).cgPath
      ring.strokeColor = UIColor.tertiaryLabel.cgColor; ring.fillColor = UIColor.clear.cgColor; ring.lineWidth = 2; let step = 2 * CGFloat.pi * 54 / 32; ring.lineDashPattern = [NSNumber(value: Double(step * 4 / 9)), NSNumber(value: Double(step * 5 / 9))]; avatar.layer.addSublayer(ring)
    }
  }
  private func tablerPlusImage() -> UIImage? {
    guard let frameworks = Bundle.main.privateFrameworksURL,
      let provider = CGDataProvider(url: frameworks.appendingPathComponent("App.framework/flutter_assets/assets/fonts/TablerIcons.ttf") as CFURL),
      let font = CGFont(provider), let fontName = font.postScriptName else { return nil }
    CTFontManagerRegisterGraphicsFont(font, nil)
    guard let uiFont = UIFont(name: fontName as String, size: 28), let scalar = UnicodeScalar(0xeb0b) else { return nil }
    let glyph = NSAttributedString(string: String(scalar), attributes: [.font: uiFont])
    let size = glyph.size()
    return UIGraphicsImageRenderer(size: CGSize(width: 32, height: 32)).image { _ in
      glyph.draw(at: CGPoint(x: (32 - size.width) / 2, y: (32 - size.height) / 2))
    }.withRenderingMode(.alwaysTemplate)
  }
  private func color(_ key: String, fallback: UIColor) -> UIColor {
    guard let value = (args[key] as? NSNumber)?.uint32Value else { return fallback }
    return UIColor(red: CGFloat((value >> 16) & 255) / 255, green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: CGFloat((value >> 24) & 255) / 255)
  }
  private func updateName() { submit.isEnabled = !draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
  private func save() {
    name.isEnabled = false; controls.forEach { $0.isEnabled = false }; navigationController?.isModalInPresentation = true; navigationItem.rightBarButtonItem?.isEnabled = false
    submit.configuration?.showsActivityIndicator = true
    action?("save", ["name": draftName]) { [weak self] response in
      guard let self else { return }
      if response["saved"] as? Bool == true { self.finish?("saved"); return }
      self.status.text = "Could not save your profile. Try again or skip."; self.status.isHidden = false
      UIAccessibility.post(notification: .announcement, argument: self.status.text)
      self.name.isEnabled = true; self.controls.forEach { $0.isEnabled = true }; self.updateName(); self.navigationController?.isModalInPresentation = false; self.navigationItem.rightBarButtonItem?.isEnabled = true; self.submit.configuration?.showsActivityIndicator = false
    }
  }
}
