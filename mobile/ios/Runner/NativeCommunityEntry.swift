import Flutter
import UIKit
import CoreText

/// One system sheet owns both the identity choices and the import detent.
final class NativeCommunityEntryCoordinator: NSObject, UIAdaptivePresentationControllerDelegate {
  private let channel: FlutterMethodChannel
  private var pending: FlutterResult?
  private var navigation: UINavigationController?
  private var session: Int?

  init(messenger: FlutterBinaryMessenger) {
    channel = FlutterMethodChannel(name: "buzz/community_entry", binaryMessenger: messenger)
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self else { return }
      guard call.method == "present" else { result(FlutterMethodNotImplemented); return }
      guard self.pending == nil else {
        result(FlutterError(code: "already_presented", message: nil, details: nil)); return
      }
      guard let args = call.arguments as? [String: Any], let session = args["session"] as? Int else {
        result(FlutterError(code: "invalid_arguments", message: nil, details: nil)); return
      }
      let root = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        .filter { $0.activationState == .foregroundActive }.flatMap(\.windows)
        .first(where: \.isKeyWindow)?.rootViewController
      guard var presenter = root else {
        result(FlutterError(code: "presentation_failed", message: nil, details: nil)); return
      }
      while let presented = presenter.presentedViewController { presenter = presented }
      let controller = NativeCommunityEntryController(configuration: args)
      controller.action = { [weak self] method, payload, completion in
        guard let self, self.session == session else { return }
        var arguments = payload
        arguments["session"] = session
        self.channel.invokeMethod(method, arguments: arguments) { value in
          guard self.session == session else { return }
          completion(value as? [String: Any] ?? ["error": "Could not finish. Try again."])
        }
      }
      controller.finish = { [weak self] joined in self?.finish(joined) }
      let navigation = UINavigationController(rootViewController: controller)
      navigation.modalPresentationStyle = .pageSheet
      navigation.overrideUserInterfaceStyle = args["dark"] as? Bool == true ? .dark : .light
      self.pending = result
      self.session = session
      self.navigation = navigation
      controller.configureDetents()
      presenter.present(navigation, animated: !UIAccessibility.isReduceMotionEnabled)
      navigation.presentationController?.delegate = self
    }
  }

  func presentationControllerWillDismiss(_ presentationController: UIPresentationController) {
    cancel()
  }

  func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
    complete(false)
  }

  private func cancel() {
    guard let session else { return }
    channel.invokeMethod("cancel", arguments: ["session": session])
  }

  private func finish(_ joined: Bool) {
    if !joined { cancel() }
    navigation?.dismiss(animated: !UIAccessibility.isReduceMotionEnabled) { [weak self] in
      self?.complete(joined)
    }
  }

  private func complete(_ joined: Bool) {
    let result = pending
    pending = nil
    navigation = nil
    session = nil
    result?(joined)
  }
}

private final class NativeCommunityEntryController: UIViewController, UITextFieldDelegate {
  var action: ((String, [String: Any], @escaping ([String: Any]) -> Void) -> Void)?
  var finish: ((Bool) -> Void)?
  private let configuration: [String: Any]
  private let stack = UIStackView()
  private let scroll = UIScrollView()
  private let input = UITextField()
  private let status = UILabel()
  private let preview = UILabel()
  private var submit: UIButton?
  private var buttons: [UIButton] = []
  private var importing = false
  private var busy = false
  private var generation = 0
  private var confirmed = false
  private var measuredHeader: CGFloat = 56
  private let compact = UISheetPresentationController.Detent.Identifier("join")
  private var invite: Bool { configuration["invite"] as? Bool == true }
  private var dark: Bool { configuration["dark"] as? Bool == true }
  private var fieldBackground: UIColor { UIColor(white: dark ? 0.16 : 0.95, alpha: 1) }
  private var choices: [[String: String]] { configuration["choices"] as? [[String: String]] ?? [] }

  init(configuration: [String: Any]) {
    self.configuration = configuration
    super.init(nibName: nil, bundle: nil)
  }
  required init?(coder: NSCoder) { nil }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = dark ? UIColor(white: 0.11, alpha: 1) : .white
    navigationController?.view.backgroundColor = view.backgroundColor
    let argb = (configuration["primaryArgb"] as? NSNumber)?.uint32Value ?? 0xFF007AFF
    view.tintColor = UIColor(red: CGFloat((argb >> 16) & 255) / 255,
      green: CGFloat((argb >> 8) & 255) / 255, blue: CGFloat(argb & 255) / 255, alpha: 1)
    navigationItem.rightBarButtonItem = UIBarButtonItem(systemItem: .close,
      primaryAction: UIAction { [weak self] _ in self?.close() })
    if #available(iOS 17.0, *) { view.keyboardLayoutGuide.usesBottomSafeArea = false }
    scroll.contentInsetAdjustmentBehavior = .never
    scroll.translatesAutoresizingMaskIntoConstraints = false
    stack.translatesAutoresizingMaskIntoConstraints = false
    stack.axis = .vertical
    stack.spacing = 12
    view.addSubview(scroll)
    scroll.addSubview(stack)
    NSLayoutConstraint.activate([
      scroll.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
      scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      scroll.bottomAnchor.constraint(equalTo: view.keyboardLayoutGuide.topAnchor),
      stack.topAnchor.constraint(equalTo: scroll.contentLayoutGuide.topAnchor, constant: 20),
      stack.leadingAnchor.constraint(equalTo: scroll.frameLayoutGuide.leadingAnchor, constant: 20),
      stack.trailingAnchor.constraint(equalTo: scroll.frameLayoutGuide.trailingAnchor, constant: -20),
      stack.bottomAnchor.constraint(equalTo: scroll.contentLayoutGuide.bottomAnchor, constant: -20),
    ])
    status.numberOfLines = 0
    status.font = .preferredFont(forTextStyle: .footnote)
    status.adjustsFontForContentSizeCategory = true
    status.textColor = .secondaryLabel
    preview.numberOfLines = 0
    preview.font = .preferredFont(forTextStyle: .footnote)
    preview.adjustsFontForContentSizeCategory = true
    input.placeholder = "Private key (nsec)"
    input.accessibilityLabel = "Private key (nsec)"
    input.isSecureTextEntry = true
    input.autocorrectionType = .no
    input.autocapitalizationType = .none
    input.spellCheckingType = .no
    input.smartQuotesType = .no
    input.smartDashesType = .no
    input.smartInsertDeleteType = .no
    input.font = .preferredFont(forTextStyle: .body)
    input.adjustsFontForContentSizeCategory = true
    input.backgroundColor = fieldBackground
    input.textColor = dark ? .white : .black
    input.layer.cornerRadius = 12
    input.leftView = UIView(frame: CGRect(x: 0, y: 0, width: 16, height: 1))
    input.leftViewMode = .always
    input.rightView = UIView(frame: CGRect(x: 0, y: 0, width: 16, height: 1))
    input.rightViewMode = .always
    input.heightAnchor.constraint(greaterThanOrEqualToConstant: 56).isActive = true
    input.delegate = self
    input.addTarget(self, action: #selector(edited), for: .editingChanged)
    render()
  }

  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    let header = max(56, view.safeAreaInsets.top,
      navigationController?.navigationBar.bounds.height ?? 0)
    if abs(header - measuredHeader) > 0.5 {
      measuredHeader = header
      navigationController?.sheetPresentationController?.invalidateDetents()
    }
  }

  func configureDetents() {
    guard let sheet = navigationController?.sheetPresentationController else { return }
    loadViewIfNeeded()
    sheet.detents = [.custom(identifier: compact) { [weak self] context in
      guard let self else { return min(320, context.maximumDetentValue) }
      let width = max(280, self.view.bounds.width - 40)
      let content = self.stack.systemLayoutSizeFitting(
        CGSize(width: width, height: UIView.layoutFittingCompressedSize.height),
        withHorizontalFittingPriority: .required, verticalFittingPriority: .fittingSizeLevel
      ).height
      // Include the actual navigation inset. Subtracting the window safe area
      // made the compact detent shorter than its visible content viewport.
      return min(content + 40 + self.measuredHeader, context.maximumDetentValue)
    }, .large()]
    sheet.selectedDetentIdentifier = importing ? .large : compact
    sheet.prefersScrollingExpandsWhenScrolledToEdge = false
    // UIKit owns the corners, drag physics, and transition between detents.
  }

  private func render() {
    title = importing ? "Import identity" : "Join community"
    stack.arrangedSubviews.forEach { stack.removeArrangedSubview($0); $0.removeFromSuperview() }
    buttons = []
    submit = nil
    status.text = nil
    status.isHidden = true
    preview.isHidden = !confirmed
    if importing {
      stack.addArrangedSubview(input)
      addButton("Choose nsec file") { [weak self] in self?.chooseFile() }
      stack.addArrangedSubview(preview)
      submit = addButton(confirmed ? "Import this identity" : "Preview identity", primary: true) {
        [weak self] in self?.submitKey()
      }
    } else {
      let name = UILabel()
      name.text = configuration["name"] as? String
      name.font = .preferredFont(forTextStyle: .headline)
      name.adjustsFontForContentSizeCategory = true
      name.numberOfLines = 0
      name.textAlignment = .center
      name.textColor = dark ? .white : .black
      let card = UIView()
      card.backgroundColor = fieldBackground
      card.layer.cornerRadius = 12
      name.translatesAutoresizingMaskIntoConstraints = false
      card.addSubview(name)
      NSLayoutConstraint.activate([
        name.topAnchor.constraint(equalTo: card.topAnchor, constant: 32),
        name.bottomAnchor.constraint(equalTo: card.bottomAnchor, constant: -32),
        name.leadingAnchor.constraint(equalTo: card.leadingAnchor, constant: 20),
        name.trailingAnchor.constraint(equalTo: card.trailingAnchor, constant: -20),
      ])
      stack.addArrangedSubview(card)
      if invite {
        addButton("Join community", primary: true) { [weak self] in
          self?.request("join", [:])
        }
      } else {
        for choice in choices {
          addButton("\(choice["label"] ?? "Saved identity")\n\(choice["npub"] ?? "")") { [weak self] in
            self?.request("join", ["pubkey": choice["pubkey"] ?? ""])
          }
        }
        addButton("Import a Nostr key") { [weak self] in self?.showImport() }
        if choices.isEmpty {
          addButton("Create an identity and join", primary: true) { [weak self] in
            self?.request("join", ["create": true])
          }
        }
      }
    }
    stack.addArrangedSubview(status)
    setBusy(false)
  }

  @discardableResult
  private func addButton(_ title: String, primary: Bool = false, action: @escaping () -> Void) -> UIButton {
    let config = onboardingButtonConfiguration(title, primary: primary)
    let button = UIButton(configuration: config, primaryAction: UIAction { _ in action() })
    button.titleLabel?.numberOfLines = 0
    button.titleLabel?.adjustsFontForContentSizeCategory = true
    button.heightAnchor.constraint(greaterThanOrEqualToConstant: 52).isActive = true
    buttons.append(button)
    stack.addArrangedSubview(button)
    return button
  }

  private func showImport() {
    importing = true
    UIView.performWithoutAnimation {
      self.render()
      self.view.layoutIfNeeded()
    }
    navigationController?.sheetPresentationController?.animateChanges {
      self.navigationController?.sheetPresentationController?.selectedDetentIdentifier = .large
    }
    // The field stays ready for a tap; keyboard appearance never competes with expansion.
  }

  private func close() {
    if invite && busy { return }
    generation += 1
    view.endEditing(true)
    input.text = nil
    preview.text = nil
    confirmed = false
    action?("cancel", [:]) { _ in }
    if importing {
      importing = false
      UIView.performWithoutAnimation {
        self.render()
        self.view.layoutIfNeeded()
      }
      navigationController?.sheetPresentationController?.animateChanges {
        self.navigationController?.sheetPresentationController?.selectedDetentIdentifier = self.compact
      }
    } else { finish?(false) }
  }

  private func setBusy(_ value: Bool) {
    busy = value
    if invite {
      navigationController?.isModalInPresentation = value
      navigationItem.rightBarButtonItem?.isEnabled = !value
    }
    buttons.forEach { $0.isEnabled = !value }
    input.isEnabled = !value
    if value { status.text = "Working…"; status.isHidden = false }
  }

  private func request(_ method: String, _ args: [String: Any], completion: (([String: Any]) -> Void)? = nil) {
    guard !busy else { return }
    generation += 1
    let attempt = generation
    setBusy(true)
    action?(method, args) { [weak self] result in
      guard let self, self.generation == attempt else { return }
      self.setBusy(false)
      self.status.text = result["error"] as? String
      self.status.isHidden = self.status.text == nil
      if let error = self.status.text {
        UIAccessibility.post(notification: .announcement, argument: error)
      }
      self.navigationController?.sheetPresentationController?.invalidateDetents()
      if result["joined"] as? Bool == true {
        self.input.text = nil
        self.finish?(true)
      } else { completion?(result) }
    }
  }

  private func showPreview(_ result: [String: Any]) {
    guard let npub = result["npub"] as? String else { return }
    confirmed = true
    preview.isHidden = false
    preview.text = "Confirm this is the public identity you want to use:\n\n\(npub)"
    submit?.configuration?.title = "Import this identity"
    view.endEditing(true)
  }

  private func submitKey() {
    request(confirmed ? "join" : "preview", ["nsec": input.text ?? ""]) { [weak self] result in
      self?.showPreview(result)
    }
  }

  private func chooseFile() {
    view.endEditing(true)
    request("file", [:]) { [weak self] result in
      guard let self, let text = result["nsec"] as? String else { return }
      self.input.text = text
      self.showPreview(result)
    }
  }

  @objc private func edited() {
    confirmed = false
    preview.text = nil
    preview.isHidden = true
    status.text = nil
    status.isHidden = true
    submit?.configuration?.title = "Preview identity"
  }

  func textField(_ textField: UITextField, shouldChangeCharactersIn range: NSRange, replacementString string: String) -> Bool {
    guard let text = textField.text, let range = Range(range, in: text) else { return false }
    return text.replacingCharacters(in: range, with: string).utf8.count <= 1024
  }
}

func onboardingButtonConfiguration(_ title: String, primary: Bool) -> UIButton.Configuration {
    var config = primary ? UIButton.Configuration.filled() : .plain()
    config.title = title
    config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attributes in
      var updated = attributes
      // Match Flutter labelMedium: bundled Inter, 14 points, weight 500.
      let inter = NativeAttachmentMenuTypography.font(forTextStyle: .subheadline)
      let descriptor = inter.fontDescriptor.addingAttributes([
        UIFontDescriptor.AttributeName(rawValue: kCTFontVariationAttribute as String):
          [NSNumber(value: 0x77676874): NSNumber(value: 500)]
      ])
      let font = UIFont(descriptor: descriptor, size: 14)
      updated.font = UIFontMetrics(forTextStyle: .subheadline).scaledFont(for: font)
      return updated
    }
    // Match the circular StadiumBorder used by the Flutter Continue button.
    config.cornerStyle = .fixed
    config.background.cornerRadius = 26
    config.contentInsets = NSDirectionalEdgeInsets(top: 14, leading: 20, bottom: 14, trailing: 20)
    if !primary {
      config.baseForegroundColor = .label
      config.background.strokeColor = .separator
      config.background.strokeWidth = 1
    }
    return config
}
