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
  private var disabledSheetPans: [UIGestureRecognizer] = []
  private var previousInteractivePopEnabled: Bool?
  private var editorRequested = false
  private var editorReady = false
  private var editorCanDone = false
  private var editorBusy = false
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
      if call.method == "appearance" {
        guard let args = call.arguments as? [String: Any], args["session"] as? Int == self.session,
              let controller = self.navigation?.viewControllers.first as? NativeProfileSetupController else { result(nil); return }
        self.navigation?.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
        controller.updateAppearance(args)
        self.updateEditorAppearance(controller)
        result(nil); return
      }
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
      controller.prepareAvatar = { [weak self, weak controller] in
        guard let self, let controller else { return }
        // Animated mode acquires the camera, so only prepare passive modes.
        guard self.avatarDraft?["type"] as? String != "animated" else { return }
        self.openAvatar(from: controller, args: args, session: session, preparing: true)
      }
      controller.closeAvatar = { [weak self, weak controller] in
        guard let self else { return }
        if self.editorRequested && self.editorReady {
          self.avatarChannel?.invokeMethod("back", arguments: nil)
        } else { self.restoreProfilePage(controller) }
      }
      controller.action = { [weak self] action, payload, completion in
        guard let self, self.session == session else { return }
        self.channel.invokeMethod(action, arguments: payload.merging(["session": session]) { _, new in new }) { value in
          guard self.session == session else { return }
          completion(value as? [String: Any] ?? [:])
        }
      }
      let nav = UINavigationController(rootViewController: controller)
      nav.modalPresentationStyle = .overFullScreen
      nav.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
      self.pending = result; self.session = session; self.navigation = nav
      controller.loadViewIfNeeded()
      presenter.present(nav, animated: !UIAccessibility.isReduceMotionEnabled) { controller.focusNameIfNeeded() }
      nav.presentationController?.delegate = self
    }
  }
  private func openAvatar(from controller: NativeProfileSetupController, args: [String: Any], session: Int, preparing: Bool = false) {
    guard self.session == session, let navigation else { return }
    if !preparing {
      UIImpactFeedbackGenerator(style: .light).impactOccurred()
      previousInteractivePopEnabled = navigation.interactivePopGestureRecognizer?.isEnabled
      navigation.isModalInPresentation = true
      var sheetView: UIView? = navigation.view
      while let current = sheetView {
        for gesture in current.gestureRecognizers ?? [] where gesture is UIPanGestureRecognizer && gesture.isEnabled {
          gesture.isEnabled = false; disabledSheetPans.append(gesture)
        }
        if current === navigation.presentationController?.containerView { break }
        sheetView = current.superview
      }
      navigation.interactivePopGestureRecognizer?.isEnabled = false
      editorRequested = true
      controller.setEditorVisible(true)
      controller.navigationItem.rightBarButtonItem = UIBarButtonItem(title: "Done", primaryAction: UIAction { [weak self] _ in self?.avatarChannel?.invokeMethod("done", arguments: nil) })
      controller.navigationItem.rightBarButtonItem?.isEnabled = editorCanDone && !editorBusy
    }
    if avatarEngine != nil {
      if !preparing && editorReady { revealEditor(controller) }
      return
    }
    controller.view.endEditing(true)
    let editorFrame = controller.editorFrame
    let avatarFrame = controller.avatarFrame(in: controller.view).offsetBy(dx: -editorFrame.minX, dy: -editorFrame.minY)
    let engine = FlutterEngine(name: "profile-avatar-\(session)", project: nil, allowHeadlessExecution: false)
    avatarEngine = engine
    guard startEditorEngine(engine) else {
      restoreProfilePage(controller); return
    }
    let editorChannel = FlutterMethodChannel(name: "buzz/profile_avatar_sheet", binaryMessenger: engine.binaryMessenger)
    avatarChannel = editorChannel
    var ready = false
    var finishing = false
    editorChannel.setMethodCallHandler { [weak self, weak controller, weak engine] call, result in
      guard let self, let engine, self.session == session, self.avatarEngine === engine, let controller else { result(nil); return }
      if call.method == "ready", self.avatarController != nil {
        ready = true
        self.editorReady = true
        self.updateEditorAppearance(controller)
        if self.editorRequested { self.revealEditor(controller) }
        result(nil)
        return
      }
      if call.method == "controls", let state = call.arguments as? [String: Any] {
        guard !finishing else { result(nil); return }
        self.editorCanDone = state["canDone"] as? Bool ?? false
        self.editorBusy = state["busy"] as? Bool ?? false
        let fullScreen = state["fullScreen"] as? Bool == true
        navigation.setNavigationBarHidden(fullScreen, animated: false)
        navigation.view.layoutIfNeeded()
        self.avatarController?.view.frame = fullScreen ? controller.view.bounds : controller.editorFrame
        controller.navigationItem.rightBarButtonItem?.isEnabled = self.editorCanDone && !self.editorBusy
        controller.navigationItem.leftBarButtonItem?.isEnabled = !self.editorBusy
        result(nil); return
      }
      if call.method == "initial" { result(["dark": controller.appearance["dark"] ?? false, "draft": self.avatarDraft ?? [:], "previewRect": ["x": avatarFrame.minX, "y": avatarFrame.minY, "width": avatarFrame.width, "height": avatarFrame.height]]); return }
      guard call.method == "finish", !finishing else { result(FlutterMethodNotImplemented); return }
      finishing = true
      let draft = call.arguments as? [String: Any]
      func returnToProfile() {
        controller.navigationItem.setRightBarButton(nil, animated: false)
        // Restore the native chrome alongside Flutter's exit. The transparent
        // editor reveals it during the animation; the final handoff must not
        // restart it or allow another navigation action mid-transition.
        controller.view.isUserInteractionEnabled = false
        controller.navigationItem.leftBarButtonItem?.isEnabled = false
        controller.setEditorVisible(false)
        let frame = controller.editorFrame
        let rect = controller.avatarFrame(in: controller.view).offsetBy(dx: -frame.minX, dy: -frame.minY)
        // Keep the moving avatar opaque until Flutter has drawn its final
        // collapsed frame, then replace it with the native avatar in one handoff.
        self.avatarChannel?.invokeMethod("hide", arguments: ["x": rect.minX, "y": rect.minY, "width": rect.width, "height": rect.height]) { [weak self, weak controller] _ in
          guard let self, self.session == session, self.avatarEngine === engine else { return }
          self.restoreProfilePage(controller)
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
    editor.isViewOpaque = false
    editor.view.backgroundColor = .clear
    controller.addChild(editor)
    editor.view.frame = editorFrame
    editor.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    editor.view.alpha = 0
    controller.view.addSubview(editor.view)
    editor.didMove(toParent: controller)
    // Recover the whole sheet when Dart never delivers the editor's first frame.
    scheduleEditorWatchdog { [weak self, weak engine, weak controller] in
      guard let self, let engine, self.session == session,
            self.avatarEngine === engine, !ready, !finishing else { return }
      self.restoreProfilePage(controller)
    }

  }

  private func updateEditorAppearance(_ controller: NativeProfileSetupController) {
    guard editorReady, let engine = avatarEngine else { return }
    FlutterMethodChannel(name: "buzz/profile_avatar_theme", binaryMessenger: engine.binaryMessenger)
      .invokeMethod("appearance", arguments: ["dark": controller.appearance["dark"] ?? false])
  }

  private func revealEditor(_ controller: NativeProfileSetupController) {
    let frame = controller.editorFrame
    avatarController?.view.frame = frame
    let rect = controller.avatarFrame(in: controller.view).offsetBy(dx: -frame.minX, dy: -frame.minY)
    avatarChannel?.invokeMethod("reveal", arguments: ["x": rect.minX, "y": rect.minY, "width": rect.width, "height": rect.height])
    // Flutter moves and grows the preview from this anchor into the editor.
    // Show it immediately and hide the native copy to avoid a crossfade ghost.
    controller.setAvatarHidden(true)
    avatarController?.view.alpha = 1
    UIAccessibility.post(notification: .screenChanged, argument: avatarController?.view)
  }

  /// Both failed startup and normal editor return must leave a retryable sheet.
  private func restoreProfilePage(_ controller: NativeProfileSetupController?) {
    editorRequested = false; editorReady = false; editorCanDone = false; editorBusy = false
    navigation?.setNavigationBarHidden(false, animated: false)
    controller?.navigationItem.rightBarButtonItem = nil
    avatarChannel?.setMethodCallHandler(nil); avatarChannel = nil
    avatarController?.willMove(toParent: nil)
    avatarController?.view.removeFromSuperview()
    avatarController?.removeFromParent()
    avatarEngine?.destroyContext(); avatarEngine = nil
    avatarController = nil
    navigation?.isModalInPresentation = false
    if let enabled = previousInteractivePopEnabled { navigation?.interactivePopGestureRecognizer?.isEnabled = enabled }
    previousInteractivePopEnabled = nil
    disabledSheetPans.forEach { $0.isEnabled = true }; disabledSheetPans.removeAll()
    controller?.view.isUserInteractionEnabled = true
    controller?.navigationItem.leftBarButtonItem?.isEnabled = true
    controller?.setEditorVisible(false)
    controller?.setAvatarHidden(false)
    controller?.avatarReady()
  }

  private func finish(_ action: String) {
    navigation?.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in self?.complete(action) }
  }
  private func complete(_ action: String) {
    editorRequested = false; editorReady = false; editorCanDone = false; editorBusy = false
    navigation?.setNavigationBarHidden(false, animated: false)
    disabledSheetPans.forEach { $0.isEnabled = true }; disabledSheetPans.removeAll()
    let result = pending; pending = nil; navigation = nil; session = nil
    avatarController?.willMove(toParent: nil)
    avatarController?.view.removeFromSuperview()
    avatarController?.removeFromParent()
    avatarChannel?.setMethodCallHandler(nil); avatarChannel = nil; avatarEngine?.destroyContext(); avatarEngine = nil; avatarController = nil; avatarDraft = nil
    result?(action)
  }
  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) { complete("skip") }
}

private final class NativeProfileSetupController: UIViewController, UITextFieldDelegate {
  var finish: ((String) -> Void)?
  var action: ((String, [String: Any], @escaping ([String: Any]) -> Void) -> Void)?
  var openAvatar: (() -> Void)?
  var prepareAvatar: (() -> Void)?
  var closeAvatar: (() -> Void)?
  private var editingAvatar = false
  private var args: [String: Any]
  var appearance: [String: Any] { args }
  private var avatarArguments: [String: Any] = [:]
  private let name = UITextField()
  private let avatar = UIButton(type: .system)
  private let status = UILabel()
  private let heading = UILabel()
  private let explanation = UILabel()
  private let explanationRow = UIStackView()
  private let avatarRow = UIView()
  private let spacer = UIView()
  private var skip: UIButton!
  private var notNow: UIButton!
  private var saving = false
  private var choosingAvatar = false
  private var hasAvatar = false
  private let avatarSize: CGFloat = 220
  private var controls: [UIButton] = []
  private var submit: UIButton!
  private var draftName: String
  init(args: [String: Any]) { self.args = args; draftName = args["name"] as? String ?? ""; super.init(nibName: nil, bundle: nil) }
  required init?(coder: NSCoder) { nil }
  override func viewDidLoad() {
    super.viewDidLoad()
    title = nil
    view.backgroundColor = color("pageBackgroundArgb", fallback: .systemBackground)
    let argb = (args["primaryArgb"] as? NSNumber)?.uint32Value ?? 0xFF007AFF
    view.tintColor = UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255, alpha: 1)
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
    let avatarMinimumHeight = avatarRow.heightAnchor.constraint(greaterThanOrEqualToConstant: avatarSize)
    avatarMinimumHeight.priority = .defaultHigh
    avatarMinimumHeight.isActive = true
    avatarRow.setContentHuggingPriority(.init(1), for: .vertical)
    avatar.translatesAutoresizingMaskIntoConstraints = false
    avatar.accessibilityLabel = "Add profile photo"
    avatar.tintColor = .label
    updateAvatar(args)
    avatar.addAction(UIAction { [weak self] _ in
      guard let self else { return }; self.avatar.isEnabled = false; self.openAvatar?()
    }, for: .touchUpInside)
    avatarRow.addSubview(avatar)
    NSLayoutConstraint.activate([avatar.centerXAnchor.constraint(equalTo: avatarRow.centerXAnchor), avatar.centerYAnchor.constraint(equalTo: avatarRow.centerYAnchor), avatar.widthAnchor.constraint(equalToConstant: avatarSize), avatar.heightAnchor.constraint(equalToConstant: avatarSize)])
    controls.append(avatar)
    heading.text = "Choose a display name"
    heading.font = .preferredFont(forTextStyle: .largeTitle)
    heading.adjustsFontForContentSizeCategory = true
    heading.numberOfLines = 0
    heading.accessibilityTraits.insert(.header)
    stack.addArrangedSubview(heading)
    name.placeholder = "Display name"; name.accessibilityLabel = "Display name"; name.text = draftName
    name.font = .preferredFont(forTextStyle: .body); name.adjustsFontForContentSizeCategory = true
    name.backgroundColor = color("containerBackgroundArgb", fallback: .secondarySystemBackground); name.layer.cornerRadius = CGFloat((args["containerCornerRadius"] as? NSNumber)?.doubleValue ?? 22)
    name.clearButtonMode = .never; name.autocapitalizationType = .words; name.returnKeyType = .done
    name.leftView = UIView(frame: CGRect(x: 0, y: 0, width: 20, height: 1)); name.leftViewMode = .always
    let nameMinimumHeight = name.heightAnchor.constraint(greaterThanOrEqualToConstant: 56)
    nameMinimumHeight.priority = .defaultHigh
    nameMinimumHeight.isActive = true
    name.delegate = self; name.addTarget(self, action: #selector(nameChanged), for: .editingChanged)
    explanation.text = "Your display name is how people see you in Buzz. You can change it any time."
    explanation.font = UIFontMetrics(forTextStyle: .subheadline).scaledFont(for: .systemFont(ofSize: 16, weight: .regular))
    explanation.adjustsFontForContentSizeCategory = true
    explanation.textColor = color("subtitleArgb", fallback: .secondaryLabel)
    explanation.numberOfLines = 0
    explanationRow.addArrangedSubview(explanation)
    explanationRow.isLayoutMarginsRelativeArrangement = true
    stack.addArrangedSubview(explanationRow)
    // Match Grid.xxs between onboarding titles and subtitles.
    stack.setCustomSpacing(8, after: heading)
    stack.addArrangedSubview(name)
    status.numberOfLines = 0; status.textColor = .systemRed; status.font = .preferredFont(forTextStyle: .footnote); status.isHidden = true; stack.addArrangedSubview(status)
    stack.addArrangedSubview(avatarRow)
    spacer.setContentHuggingPriority(.init(1), for: .vertical)
    stack.addArrangedSubview(spacer)
    var continueConfiguration = onboardingButtonConfiguration("Continue", primary: true)
    continueConfiguration.baseForegroundColor = color("onPrimaryArgb", fallback: args["dark"] as? Bool == true ? .black : .white)
    submit = UIButton(configuration: continueConfiguration, primaryAction: UIAction { [weak self] _ in self?.advance() })
    var skipConfiguration = onboardingButtonConfiguration("Skip", primary: false)
    skipConfiguration.background.strokeWidth = 0
    skipConfiguration.background.strokeColor = .clear
    skip = UIButton(configuration: skipConfiguration, primaryAction: UIAction { [weak self] _ in self?.save(skipAvatar: true) })
    notNow = UIButton(configuration: onboardingButtonConfiguration("Not now", primary: false), primaryAction: UIAction { [weak self] _ in self?.finish?("skip") })
    notNow.isHidden = true
    for button in [submit!, skip!, notNow!] {
      let minimumHeight = button.heightAnchor.constraint(greaterThanOrEqualToConstant: 52)
      minimumHeight.priority = .defaultHigh
      minimumHeight.isActive = true
      stack.addArrangedSubview(button); controls.append(button)
    }
    // Match Grid.xxs between primary and ghost actions in pairing onboarding.
    stack.setCustomSpacing(8, after: submit)
    showStep(avatar: false, announce: false)
  }
  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    focusNameIfNeeded()
  }
  func focusNameIfNeeded() {
    if !choosingAvatar { name.becomeFirstResponder() }
  }
  private func showStep(avatar: Bool, announce: Bool = true) {
    choosingAvatar = avatar
    view.endEditing(true)
    heading.text = avatar ? "Choose your avatar" : "Choose a display name"
    explanation.text = avatar
      ? "Let’s choose an avatar so people know it’s you."
      : "Your display name is how people see you in Buzz. You can change it any time."
    // Mobile bodyLarge subtitle: 16/20, regular, subtle.
    let paragraph = NSMutableParagraphStyle()
    paragraph.minimumLineHeight = UIFontMetrics(forTextStyle: .subheadline).scaledValue(for: 20)
    explanation.attributedText = NSAttributedString(string: explanation.text ?? "", attributes: [
      .paragraphStyle: paragraph,
      .kern: 0
    ])
    explanationRow.directionalLayoutMargins = .zero
    name.isHidden = avatar
    avatarRow.isHidden = !avatar
    spacer.isHidden = avatar
    skip.isHidden = !avatar
    status.isHidden = true
    notNow.isHidden = true
    navigationItem.leftBarButtonItem = avatar
      ? UIBarButtonItem(title: "Back", image: UIImage(systemName: "chevron.backward"), primaryAction: UIAction { [weak self] _ in self?.goBack() })
      : nil
    updateName()
    if avatar {
      view.layoutIfNeeded()
      prepareAvatar?()
    }
    if announce {
      UIAccessibility.post(notification: .screenChanged, argument: heading)
      if !avatar { name.becomeFirstResponder() }
    }
  }
  private func goBack() {
    if editingAvatar { closeAvatar?() } else {
      closeAvatar?()
      showStep(avatar: false)
    }
  }
  private func advance() {
    guard !draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    if choosingAvatar { save() } else { showStep(avatar: true) }
  }
  @objc private func nameChanged() { draftName = name.text ?? ""; updateName() }
  func textFieldShouldReturn(_ textField: UITextField) -> Bool { textField.resignFirstResponder(); return true }
  func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
    guard let current = textField.text, let range = Range(range, in: current) else { return false }
    return current.replacingCharacters(in: range, with: string).count <= 100
  }
  var editorFrame: CGRect {
    view.layoutIfNeeded()
    let top = view.safeAreaLayoutGuide.layoutFrame.minY + 20
    return CGRect(x: 0, y: top, width: view.bounds.width, height: max(0, view.safeAreaLayoutGuide.layoutFrame.maxY - top))
  }
  func setEditorVisible(_ visible: Bool) {
    guard editingAvatar != visible else { return }
    editingAvatar = visible
    heading.accessibilityElementsHidden = visible
    explanationRow.accessibilityElementsHidden = visible
    avatarRow.accessibilityElementsHidden = visible
    submit.accessibilityElementsHidden = visible
    skip.accessibilityElementsHidden = visible
    notNow.accessibilityElementsHidden = visible
    let motion = visible && !UIAccessibility.isReduceMotionEnabled
    // Clear the heading quickly before the segmented control replaces it.
    UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : (visible ? 0.06 : 0.2), delay: 0, options: [.beginFromCurrentState, .curveEaseOut]) {
      self.heading.alpha = visible ? 0 : 1
      self.explanationRow.alpha = visible ? 0 : 1
    }
    UIView.animate(withDuration: UIAccessibility.isReduceMotionEnabled ? 0 : 0.2, delay: 0, options: [.beginFromCurrentState, .curveEaseOut]) {
      self.heading.transform = motion ? CGAffineTransform(translationX: 0, y: -24).scaledBy(x: 0.96, y: 0.96) : .identity
      self.explanationRow.transform = self.heading.transform
      self.submit.alpha = visible ? 0 : 1
      self.skip.alpha = visible ? 0 : 1
      self.notNow.alpha = visible ? 0 : 1
      self.submit.transform = motion ? CGAffineTransform(translationX: 0, y: 40).scaledBy(x: 0.96, y: 0.96) : .identity
      self.skip.transform = self.submit.transform
      self.notNow.transform = self.submit.transform
    }
  }

  func setAvatarHidden(_ hidden: Bool) { avatar.isHidden = hidden }
  func avatarFrame(in container: UIView) -> CGRect { avatar.convert(avatar.bounds, to: container) }
  func avatarSnapshot(in container: UIView) -> (UIView, CGRect)? {
    guard let snapshot = avatar.snapshotView(afterScreenUpdates: false) else { return nil }
    return (snapshot, avatar.convert(avatar.bounds, to: container))
  }
  func avatarReady() { avatar.isEnabled = true }
  func updateAvatar(_ args: [String: Any]) {
    avatarArguments = args
    hasAvatar = false
    defer {
      avatar.transform = hasAvatar ? .identity : CGAffineTransform(scaleX: 0.85, y: 0.85)
      avatar.accessibilityLabel = hasAvatar ? "Edit profile photo" : "Add profile photo"
      if submit != nil { updateName() }
    }
    avatar.layer.sublayers?.filter { $0 is CAShapeLayer }.forEach { $0.removeFromSuperlayer() }
    avatar.setImage(nil, for: .normal); avatar.setTitle(nil, for: .normal); avatar.backgroundColor = .clear
    if let bytes = args["avatar"] as? FlutterStandardTypedData, let image = UIImage(data: bytes.data) {
      hasAvatar = true
      avatar.setImage(avatarPreview(image: image), for: .normal); avatar.imageView?.contentMode = .scaleAspectFit
      avatar.clipsToBounds = false; avatar.layer.cornerRadius = 0
    } else if let emoji = args["emoji"] as? String {
      hasAvatar = true
      let argb = (args["emojiColor"] as? NSNumber)?.uint32Value ?? 0xFFFFFFFF
      let background = UIColor(red: CGFloat((argb >> 16) & 255) / 255, green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255, alpha: 1)
      avatar.setImage(avatarPreview(emoji: emoji, background: background), for: .normal)
      avatar.imageView?.contentMode = .scaleAspectFit
      avatar.clipsToBounds = false; avatar.layer.cornerRadius = 0
    } else {
      avatar.setImage(tablerPlusImage(), for: .normal)
      let ring = CAShapeLayer(); ring.path = UIBezierPath(ovalIn: CGRect(x: 2, y: 2, width: avatarSize - 4, height: avatarSize - 4)).cgPath
      ring.strokeColor = UIColor.tertiaryLabel.cgColor; ring.fillColor = UIColor.clear.cgColor; ring.lineWidth = 3; let step = CGFloat.pi * (avatarSize - 4) / 32; ring.lineDashPattern = [NSNumber(value: Double(step * 4 / 9)), NSNumber(value: Double(step * 5 / 9))]; avatar.layer.addSublayer(ring)
    }
  }
  func updateAppearance(_ appearance: [String: Any]) {
    args.merge(appearance) { _, new in new }
    view.backgroundColor = color("pageBackgroundArgb", fallback: .systemBackground)
    view.tintColor = color("primaryArgb", fallback: .label)
    name.backgroundColor = color("containerBackgroundArgb", fallback: .secondarySystemBackground)
    explanation.textColor = color("subtitleArgb", fallback: .secondaryLabel)
    submit.configuration?.baseBackgroundColor = view.tintColor
    submit.configuration?.baseForegroundColor = color("onPrimaryArgb", fallback: .systemBackground)
    updateAvatar(avatarArguments)
  }

  /// Mirrors Flutter's 220-point preview and AvatarBadgeMaskGeometry.badge.
  private func avatarPreview(image: UIImage? = nil, emoji: String? = nil, background: UIColor = .clear) -> UIImage {
    UIGraphicsImageRenderer(size: CGSize(width: avatarSize, height: avatarSize)).image { renderer in
      let bounds = CGRect(x: 0, y: 0, width: avatarSize, height: avatarSize)
      renderer.cgContext.saveGState()
      avatarBadgePath().addClip()
      background.setFill(); UIRectFill(bounds)
      if let image {
        let scale = max(avatarSize / image.size.width, avatarSize / image.size.height)
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        image.draw(in: CGRect(x: (avatarSize - size.width) / 2, y: (avatarSize - size.height) / 2, width: size.width, height: size.height))
      }
      if let emoji {
        // Same font/optical box ratio as _EmojiAvatarPreview, including wide sequences.
        let box = avatarSize * 258 / 512
        let text = NSAttributedString(string: emoji, attributes: [.font: UIFont.systemFont(ofSize: box)])
        let size = text.size()
        let scale = min(1, box / size.width)
        renderer.cgContext.saveGState()
        renderer.cgContext.translateBy(x: (avatarSize - size.width * scale) / 2, y: (avatarSize - size.height * scale) / 2 - 1)
        renderer.cgContext.scaleBy(x: scale, y: scale)
        text.draw(at: .zero)
        renderer.cgContext.restoreGState()
      }
      renderer.cgContext.restoreGState()
      let diameter = avatarSize * 54 / 192
      let center = avatarSize * 165 / 192
      let badge = CGRect(x: center - diameter / 2, y: center - diameter / 2, width: diameter, height: diameter)
      color("primaryArgb", fallback: .label).setFill()
      UIBezierPath(ovalIn: badge).fill()
      pencilImage(size: diameter * 0.48)
        .withTintColor(color("onPrimaryArgb", fallback: .systemBackground), renderingMode: .alwaysOriginal)
        .draw(in: badge.insetBy(dx: diameter * 0.26, dy: diameter * 0.26))
    }.withRenderingMode(.alwaysOriginal)
  }

  /// Port of shared/widgets/masked_avatar_badge.dart, including its rounded joiners.
  private func avatarBadgePath() -> UIBezierPath {
    let radius = avatarSize / 2, cutoutRadius = avatarSize * 30 / 192
    let center = CGPoint(x: radius, y: radius)
    let cutout = CGPoint(x: avatarSize * 165 / 192, y: avatarSize * 165 / 192)
    let distance = hypot(cutout.x - center.x, cutout.y - center.y)
    let midpointDistance = (radius * radius - cutoutRadius * cutoutRadius + distance * distance) / (2 * distance)
    let halfChord = sqrt(max(0, radius * radius - midpointDistance * midpointDistance))
    let unit = (cutout.x - center.x) / distance
    let upper = CGPoint(x: center.x + unit * (midpointDistance + halfChord), y: center.y + unit * (midpointDistance - halfChord))
    let lower = CGPoint(x: upper.y, y: upper.x)
    let rounding = min(0.38, max(0.22, 5.5 / cutoutRadius))
    let au = atan2(upper.y - center.y, upper.x - center.x) - 0.075
    let al = atan2(lower.y - center.y, lower.x - center.x) + 0.075
    let cu = atan2(upper.y - cutout.y, upper.x - cutout.x) - rounding
    let cl = atan2(lower.y - cutout.y, lower.x - cutout.x) + rounding
    func point(_ c: CGPoint, _ r: CGFloat, _ a: CGFloat) -> CGPoint { CGPoint(x: c.x + r * cos(a), y: c.y + r * sin(a)) }
    func tangent(_ p: CGPoint, _ a: CGFloat, _ length: CGFloat) -> CGPoint { CGPoint(x: p.x - sin(a) * length, y: p.y + cos(a) * length) }
    let aUpper = point(center, radius, au), aLower = point(center, radius, al)
    let cUpper = point(cutout, cutoutRadius, cu), cLower = point(cutout, cutoutRadius, cl)
    let uh = min(cutoutRadius * 0.14, hypot(cUpper.x - aUpper.x, cUpper.y - aUpper.y) * 0.42)
    let lh = min(cutoutRadius * 0.14, hypot(cLower.x - aLower.x, cLower.y - aLower.y) * 0.42)
    let path = UIBezierPath()
    path.move(to: cUpper)
    path.addCurve(to: aUpper, controlPoint1: tangent(cUpper, cu, uh), controlPoint2: tangent(aUpper, au, uh))
    path.addArc(withCenter: center, radius: radius, startAngle: au, endAngle: al, clockwise: false)
    path.addCurve(to: cLower, controlPoint1: tangent(aLower, al, -lh), controlPoint2: tangent(cLower, cl, -lh))
    path.addArc(withCenter: cutout, radius: cutoutRadius, startAngle: cl, endAngle: cu, clockwise: true)
    path.close()
    return path
  }

  // Tabler pencil's 24-point outline. Native-only font glyphs can be removed
  // by Flutter's release font subsetting, so render this badge as a vector.
  private func pencilImage(size: CGFloat) -> UIImage {
    UIGraphicsImageRenderer(size: CGSize(width: size, height: size)).image { renderer in
      renderer.cgContext.scaleBy(x: size / 24, y: size / 24)
      let path = UIBezierPath()
      path.move(to: CGPoint(x: 4, y: 20))
      path.addLine(to: CGPoint(x: 8, y: 20))
      path.addLine(to: CGPoint(x: 18.5, y: 9.5))
      path.addArc(withCenter: CGPoint(x: 16.5, y: 7.5), radius: sqrt(8), startAngle: .pi / 4, endAngle: -.pi * 3 / 4, clockwise: false)
      path.addLine(to: CGPoint(x: 4, y: 16))
      path.close()
      path.move(to: CGPoint(x: 13.5, y: 6.5))
      path.addLine(to: CGPoint(x: 17.5, y: 10.5))
      path.lineWidth = 2; path.lineCapStyle = .round; path.lineJoinStyle = .round
      UIColor.black.setStroke(); path.stroke()
    }.withRenderingMode(.alwaysTemplate)
  }

  private func tablerPlusImage() -> UIImage? { tablerImage(code: 0xeb0b, size: 40) }
  private func tablerImage(code: UInt32, size: CGFloat) -> UIImage? {
    guard let frameworks = Bundle.main.privateFrameworksURL,
      let provider = CGDataProvider(url: frameworks.appendingPathComponent("App.framework/flutter_assets/assets/fonts/TablerIcons.ttf") as CFURL),
      let font = CGFont(provider), let fontName = font.postScriptName else { return nil }
    CTFontManagerRegisterGraphicsFont(font, nil)
    guard let uiFont = UIFont(name: fontName as String, size: size), let scalar = UnicodeScalar(code) else { return nil }
    let glyph = NSAttributedString(string: String(scalar), attributes: [.font: uiFont])
    let measured = glyph.size()
    return UIGraphicsImageRenderer(size: CGSize(width: size, height: size)).image { _ in
      glyph.draw(at: CGPoint(x: (size - measured.width) / 2, y: (size - measured.height) / 2))
    }.withRenderingMode(.alwaysTemplate)
  }
  private func color(_ key: String, fallback: UIColor) -> UIColor {
    guard let value = (args[key] as? NSNumber)?.uint32Value else { return fallback }
    return UIColor(red: CGFloat((value >> 16) & 255) / 255, green: CGFloat((value >> 8) & 255) / 255, blue: CGFloat(value & 255) / 255, alpha: CGFloat((value >> 24) & 255) / 255)
  }
  private func updateName() { submit.isEnabled = !saving && !draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && (!choosingAvatar || hasAvatar) }
  private func save(skipAvatar: Bool = false) {
    guard choosingAvatar, skipAvatar || hasAvatar, !draftName.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    saving = true
    name.isEnabled = false; controls.forEach { $0.isEnabled = false }; navigationController?.isModalInPresentation = true; navigationItem.rightBarButtonItem?.isEnabled = false; navigationItem.leftBarButtonItem?.isEnabled = false
    submit.configuration?.showsActivityIndicator = true
    action?("save", ["name": draftName, "skipAvatar": skipAvatar]) { [weak self] response in
      guard let self else { return }
      if response["saved"] as? Bool == true { self.finish?("saved"); return }
      self.saving = false
      self.notNow.isHidden = false
      self.status.text = "Could not save your profile. Try again or choose Not now to continue without saving."; self.status.isHidden = false
      UIAccessibility.post(notification: .announcement, argument: self.status.text)
      self.name.isEnabled = true; self.controls.forEach { $0.isEnabled = true }; self.updateName(); self.navigationController?.isModalInPresentation = false; self.navigationItem.rightBarButtonItem?.isEnabled = true; self.navigationItem.leftBarButtonItem?.isEnabled = true; self.submit.configuration?.showsActivityIndicator = false
    }
  }
}
