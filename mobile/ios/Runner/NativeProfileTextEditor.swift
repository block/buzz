import Flutter
import UIKit

final class NativeProfileTextEditorCoordinator: NSObject,
  UIAdaptivePresentationControllerDelegate
{
  private let channel: FlutterMethodChannel
  private weak var parentViewController: UIViewController?
  private weak var presentedController: UIViewController?
  private var pendingResult: FlutterResult?

  init(
    messenger: FlutterBinaryMessenger,
    parentViewController: UIViewController?
  ) {
    channel = FlutterMethodChannel(
      name: "buzz/profile_text_editor",
      binaryMessenger: messenger
    )
    self.parentViewController = parentViewController
    super.init()
    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  private func handle(
    _ call: FlutterMethodCall,
    result: @escaping FlutterResult
  ) {
    guard call.method == "present" || call.method == "presentChannel" else {
      result(FlutterMethodNotImplemented)
      return
    }
    guard
      let arguments = call.arguments as? [String: Any],
      let title = arguments["title"] as? String,
      let initialValue = arguments["initialValue"] as? String,
      let placeholder = arguments["placeholder"] as? String,
      let multiline = arguments["multiline"] as? Bool,
      let brightness = arguments["brightness"] as? String,
      let pageBackgroundArgb = arguments["pageBackgroundArgb"] as? NSNumber,
      let containerBackgroundArgb = arguments["containerBackgroundArgb"] as? NSNumber,
      let containerCornerRadius = arguments["containerCornerRadius"] as? NSNumber
    else {
      result(
        FlutterError(
          code: "invalid_arguments",
          message: "Expected profile text editor configuration.",
          details: nil
        )
      )
      return
    }
    let allowUnchangedSubmission =
      arguments["allowUnchangedSubmission"] as? Bool ?? false

    DispatchQueue.main.async { [weak self] in
      self?.present(
        title: title,
        initialValue: initialValue,
        placeholder: placeholder,
        multiline: multiline,
        brightness: brightness,
        pageBackgroundColor: UIColor(argb: pageBackgroundArgb.uint32Value),
        containerBackgroundColor: UIColor(argb: containerBackgroundArgb.uint32Value),
        containerCornerRadius: CGFloat(containerCornerRadius.doubleValue),
        allowUnchangedSubmission: allowUnchangedSubmission,
        channelDescription: call.method == "presentChannel" ? arguments["description"] as? String : nil,
        originalName: arguments["originalName"] as? String,
        originalDescription: arguments["originalDescription"] as? String,
        canEditDetails: arguments["canEditDetails"] as? Bool ?? true,
        canEditCanvas: arguments["canEditCanvas"] as? Bool ?? false,
        canvasContent: arguments["canvasContent"] as? String ?? "",
        canvasLoaded: arguments["canvasLoaded"] as? Bool ?? false,
        result: result
      )
    }
  }

  @MainActor
  private func present(
    title: String,
    initialValue: String,
    placeholder: String,
    multiline: Bool,
    brightness: String,
    pageBackgroundColor: UIColor,
    containerBackgroundColor: UIColor,
    containerCornerRadius: CGFloat,
    allowUnchangedSubmission: Bool,
    channelDescription: String?,
    originalName: String?,
    originalDescription: String?,
    canEditDetails: Bool,
    canEditCanvas: Bool,
    canvasContent: String,
    canvasLoaded: Bool,
    result: @escaping FlutterResult
  ) {
    guard presentedController == nil else {
      result(
        FlutterError(
          code: "already_presented",
          message: "A profile editor is already open.",
          details: nil
        )
      )
      return
    }
    guard
      let presenter = topViewController(
        from: parentViewController ?? activeWindowRootViewController()
      )
    else {
      result(
        FlutterError(
          code: "presentation_failed",
          message: "Unable to find a view controller for the profile editor.",
          details: nil
        )
      )
      return
    }

    let editor = NativeProfileTextEditorViewController(
      title: title,
      initialValue: initialValue,
      placeholder: placeholder,
      multiline: multiline,
      pageBackgroundColor: pageBackgroundColor,
      containerBackgroundColor: containerBackgroundColor,
      containerCornerRadius: containerCornerRadius,
      allowUnchangedSubmission: allowUnchangedSubmission,
      channelDescription: channelDescription,
      originalName: originalName,
      originalDescription: originalDescription,
      canEditDetails: canEditDetails,
      canEditCanvas: canEditCanvas,
      canvasContent: canvasContent,
      canvasLoaded: canvasLoaded,
      onCancel: { [weak self] in self?.finish(value: nil) },
      onSet: { [weak self] value in self?.finish(value: value) }
    )
    let navigationController = UINavigationController(rootViewController: editor)
    navigationController.overrideUserInterfaceStyle = brightness == "dark"
      ? .dark
      : .light
    if UIDevice.current.userInterfaceIdiom == .pad {
      navigationController.modalPresentationStyle = .formSheet
    }

    if channelDescription != nil {
      navigationController.sheetPresentationController?.detents = [.large()]
    }

    pendingResult = result
    presentedController = navigationController
    presenter.present(navigationController, animated: true) { [weak self] in
      navigationController.presentationController?.delegate = self
    }
  }

  @MainActor
  private func finish(value: Any?) {
    guard let controller = presentedController else {
      resolve(value: value)
      return
    }
    controller.dismiss(animated: true) { [weak self] in
      self?.resolve(value: value)
    }
  }

  func presentationControllerDidDismiss(
    _ presentationController: UIPresentationController
  ) {
    resolve(value: nil)
  }

  @MainActor
  private func resolve(value: Any?) {
    let result = pendingResult
    pendingResult = nil
    presentedController = nil
    result?(value)
  }

  @MainActor
  private func activeWindowRootViewController() -> UIViewController? {
    UIApplication.shared.connectedScenes
      .compactMap { $0 as? UIWindowScene }
      .filter { $0.activationState == .foregroundActive }
      .flatMap(\.windows)
      .first(where: \.isKeyWindow)?
      .rootViewController
  }

  @MainActor
  private func topViewController(
    from viewController: UIViewController?
  ) -> UIViewController? {
    if let navigationController = viewController as? UINavigationController {
      return topViewController(from: navigationController.visibleViewController)
    }
    if let tabBarController = viewController as? UITabBarController {
      return topViewController(from: tabBarController.selectedViewController)
    }
    if let presentedViewController = viewController?.presentedViewController {
      return topViewController(from: presentedViewController)
    }
    return viewController
  }
}

private final class NativeProfileTextEditorViewController:
  UITableViewController,
  UITextFieldDelegate,
  UITextViewDelegate
{
  private let initialValue: String
  private let placeholder: String
  private let multiline: Bool
  private let pageBackgroundColor: UIColor
  private let containerBackgroundColor: UIColor
  private let containerCornerRadius: CGFloat
  private let allowUnchangedSubmission: Bool
  private let onCancel: () -> Void
  private let onSet: (Any) -> Void
  private let channelDescription: String?
  private let originalName: String?
  private let originalDescription: String?
  private let canEditDetails: Bool
  private let canEditCanvas: Bool
  private let canvasContent: String
  private let canvasLoaded: Bool
  private var hasCanvas: Bool {
    canvasLoaded && !canvasContent.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }
  private var isChannelEditor: Bool { channelDescription != nil }

  private lazy var textField: UITextField = {
    let field = UITextField()
    field.translatesAutoresizingMaskIntoConstraints = false
    field.placeholder = placeholder
    field.text = initialValue
    field.font = .preferredFont(forTextStyle: .body)
    field.adjustsFontForContentSizeCategory = true
    field.clearButtonMode = .whileEditing
    field.returnKeyType = isChannelEditor ? .next : .done
    field.isEnabled = canEditDetails
    field.accessibilityLabel = placeholder
    field.autocapitalizationType = .sentences
    field.delegate = self
    field.addTarget(self, action: #selector(textDidChange), for: .editingChanged)
    return field
  }()

  private lazy var textView: UITextView = {
    let view = UITextView()
    view.translatesAutoresizingMaskIntoConstraints = false
    view.text = channelDescription ?? initialValue
    view.isEditable = canEditDetails
    view.accessibilityLabel = isChannelEditor ? "Description" : placeholder
    view.font = .preferredFont(forTextStyle: .body)
    view.adjustsFontForContentSizeCategory = true
    view.backgroundColor = .clear
    view.textContainerInset = .zero
    view.textContainer.lineFragmentPadding = 0
    view.autocapitalizationType = .sentences
    view.delegate = self
    return view
  }()

  private lazy var placeholderLabel: UILabel = {
    let label = UILabel()
    label.translatesAutoresizingMaskIntoConstraints = false
    label.text = isChannelEditor ? "Description" : placeholder
    label.font = .preferredFont(forTextStyle: .body)
    label.adjustsFontForContentSizeCategory = true
    label.textColor = .placeholderText
    label.numberOfLines = 0
    return label
  }()

  init(
    title: String,
    initialValue: String,
    placeholder: String,
    multiline: Bool,
    pageBackgroundColor: UIColor,
    containerBackgroundColor: UIColor,
    containerCornerRadius: CGFloat,
    allowUnchangedSubmission: Bool,
    channelDescription: String?,
    originalName: String?,
    originalDescription: String?,
    canEditDetails: Bool,
    canEditCanvas: Bool,
    canvasContent: String,
    canvasLoaded: Bool,
    onCancel: @escaping () -> Void,
    onSet: @escaping (Any) -> Void
  ) {
    self.initialValue = initialValue
    self.placeholder = placeholder
    self.multiline = multiline
    self.pageBackgroundColor = pageBackgroundColor
    self.containerBackgroundColor = containerBackgroundColor
    self.containerCornerRadius = containerCornerRadius
    self.allowUnchangedSubmission = allowUnchangedSubmission
    self.channelDescription = channelDescription
    self.originalName = originalName
    self.originalDescription = originalDescription
    self.canEditDetails = canEditDetails
    self.canEditCanvas = canEditCanvas
    self.canvasContent = canvasContent
    self.canvasLoaded = canvasLoaded
    self.onCancel = onCancel
    self.onSet = onSet
    super.init(style: .insetGrouped)
    self.title = title
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    fatalError("init(coder:) has not been implemented")
  }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = pageBackgroundColor
    tableView.backgroundColor = pageBackgroundColor
    tableView.keyboardDismissMode = .interactive
    tableView.alwaysBounceVertical = false
    let navigationAppearance = UINavigationBarAppearance()
    navigationAppearance.configureWithOpaqueBackground()
    navigationAppearance.backgroundColor = pageBackgroundColor
    navigationAppearance.shadowColor = UIColor.separator.withAlphaComponent(0.2)
    let scrollEdgeAppearance = UINavigationBarAppearance()
    scrollEdgeAppearance.configureWithOpaqueBackground()
    scrollEdgeAppearance.backgroundColor = pageBackgroundColor
    scrollEdgeAppearance.shadowColor = .clear
    navigationItem.standardAppearance = navigationAppearance
    navigationItem.scrollEdgeAppearance = scrollEdgeAppearance
    navigationItem.compactAppearance = navigationAppearance
    navigationItem.compactScrollEdgeAppearance = scrollEdgeAppearance
    navigationItem.leftBarButtonItem = UIBarButtonItem(
      barButtonSystemItem: .cancel,
      target: self,
      action: #selector(cancelTapped)
    )
    navigationItem.rightBarButtonItem = UIBarButtonItem(
      title: isChannelEditor ? "Save" : "Set",
      style: .done,
      target: self,
      action: #selector(setTapped)
    )
    updateNavigation()
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    guard canEditDetails else { return }
    if multiline {
      textView.becomeFirstResponder()
    } else {
      textField.becomeFirstResponder()
    }
  }

  private var currentValue: String {
    multiline ? textView.text : (textField.text ?? "")
  }

  private var hasUnsavedChanges: Bool {
    if isChannelEditor {
      return allowUnchangedSubmission
        || currentValue.trimmingCharacters(in: .whitespacesAndNewlines)
          != (originalName ?? initialValue).trimmingCharacters(in: .whitespacesAndNewlines)
        || textView.text.trimmingCharacters(in: .whitespacesAndNewlines)
          != (originalDescription ?? channelDescription ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }
    return allowUnchangedSubmission
      || currentValue.trimmingCharacters(in: .whitespacesAndNewlines)
      != initialValue.trimmingCharacters(in: .whitespacesAndNewlines)
  }

  override var isModalInPresentation: Bool {
    get { hasUnsavedChanges }
    set {}
  }

  @objc private func textDidChange() {
    updateNavigation()
  }

  func textViewDidChange(_ textView: UITextView) {
    updateNavigation()
  }

  private func updateNavigation() {
    navigationItem.rightBarButtonItem?.isEnabled = canEditDetails && hasUnsavedChanges
      && (!isChannelEditor || !canonicalChannelName.isEmpty)
    navigationController?.isModalInPresentation = hasUnsavedChanges
    placeholderLabel.isHidden = !textView.text.isEmpty
  }

  @objc private func cancelTapped() {
    guard hasUnsavedChanges else {
      onCancel()
      return
    }
    let alert = UIAlertController(
      title: "Discard changes?",
      message: nil,
      preferredStyle: .actionSheet
    )
    alert.addAction(UIAlertAction(title: "Keep Editing", style: .cancel))
    alert.addAction(
      UIAlertAction(title: "Discard", style: .destructive) { [weak self] _ in
        self?.onCancel()
      }
    )
    if let popover = alert.popoverPresentationController {
      popover.barButtonItem = navigationItem.leftBarButtonItem
    }
    present(alert, animated: true)
  }

  private var canonicalChannelName: String {
    currentValue.trimmingCharacters(in: .whitespacesAndNewlines)
      .replacingOccurrences(of: "^#+", with: "", options: .regularExpression)
      .trimmingCharacters(in: .whitespacesAndNewlines)
  }

  @objc private func setTapped() {
    guard canEditDetails && hasUnsavedChanges else { return }
    if isChannelEditor {
      guard !canonicalChannelName.isEmpty else { return }
      onSet(["action": "save", "name": canonicalChannelName, "description": textView.text ?? ""])
    } else {
      onSet(currentValue)
    }
  }

  @objc private func editCanvasTapped() {
    guard canEditCanvas else { return }
    onSet(["action": "canvas", "name": currentValue, "description": textView.text ?? ""])
  }

  private func removeCanvasTapped() {
    guard canEditCanvas && hasCanvas else { return }
    let alert = UIAlertController(
      title: "Remove canvas?",
      message: "This removes the canvas content for everyone in this channel.",
      preferredStyle: .alert
    )
    alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
    alert.addAction(UIAlertAction(title: "Remove", style: .destructive) { [weak self] _ in
      guard let self else { return }
      self.onSet(["action": "removeCanvas", "name": self.currentValue, "description": self.textView.text ?? ""])
    })
    present(alert, animated: true)
  }

  override func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
    tableView.deselectRow(at: indexPath, animated: true)
    guard isChannelEditor && canEditCanvas else { return }
    if indexPath.section == 2 { editCanvasTapped() }
    if indexPath.section == 3 { removeCanvasTapped() }
  }

  func textFieldShouldReturn(_ textField: UITextField) -> Bool {
    if isChannelEditor {
      textView.becomeFirstResponder()
    } else {
      setTapped()
    }
    return false
  }

  override func numberOfSections(in tableView: UITableView) -> Int { isChannelEditor ? (hasCanvas && canEditCanvas ? 4 : 3) : 1 }

  override func tableView(
    _ tableView: UITableView,
    numberOfRowsInSection section: Int
  ) -> Int { 1 }

  override func tableView(
    _ tableView: UITableView,
    heightForRowAt indexPath: IndexPath
  ) -> CGFloat {
    if isChannelEditor && indexPath.section >= 2 { return UITableView.automaticDimension }
    return (isChannelEditor ? indexPath.section == 1 : multiline) ? 132 : 52
  }

  override func tableView(
    _ tableView: UITableView,
    cellForRowAt indexPath: IndexPath
  ) -> UITableViewCell {
    let cell = UITableViewCell(style: .default, reuseIdentifier: nil)
    cell.selectionStyle = .none
    cell.backgroundColor = containerBackgroundColor
    cell.layer.cornerRadius = containerCornerRadius
    cell.layer.cornerCurve = .continuous
    cell.clipsToBounds = true
    if isChannelEditor && indexPath.section >= 2 {
      var content = cell.defaultContentConfiguration()
      content.textProperties.font = .preferredFont(forTextStyle: .body)
      if indexPath.section == 3 {
        content.text = "Remove Canvas"
        content.textProperties.color = .systemRed
      } else if hasCanvas {
        content.text = "Canvas"
        content.secondaryText = String(canvasContent.prefix(600))
        content.secondaryTextProperties.font = .preferredFont(forTextStyle: .body)
        content.secondaryTextProperties.numberOfLines = 4
        cell.accessoryType = canEditCanvas ? .disclosureIndicator : .none
        cell.accessibilityHint = canEditCanvas ? "Opens the canvas editor" : nil
      } else {
        content.text = canvasLoaded ? "Add Canvas" : "Retry loading canvas"
        content.textProperties.color = canEditCanvas ? .label : .secondaryLabel
        cell.accessoryType = canEditCanvas ? .disclosureIndicator : .none
      }
      cell.contentConfiguration = content
      cell.selectionStyle = canEditCanvas ? .default : .none
      cell.isUserInteractionEnabled = canEditCanvas
      cell.accessibilityTraits = canEditCanvas ? .button : .staticText
    } else if isChannelEditor ? indexPath.section == 1 : multiline {
      cell.contentView.addSubview(textView)
      textView.addSubview(placeholderLabel)
      NSLayoutConstraint.activate([
        textView.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
        textView.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
        textView.topAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.topAnchor),
        textView.bottomAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.bottomAnchor),
        placeholderLabel.leadingAnchor.constraint(equalTo: textView.leadingAnchor),
        placeholderLabel.trailingAnchor.constraint(equalTo: textView.trailingAnchor),
        placeholderLabel.topAnchor.constraint(equalTo: textView.topAnchor),
      ])
      placeholderLabel.isHidden = !textView.text.isEmpty
    } else {
      cell.contentView.addSubview(textField)
      NSLayoutConstraint.activate([
        textField.leadingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.leadingAnchor),
        textField.trailingAnchor.constraint(equalTo: cell.contentView.layoutMarginsGuide.trailingAnchor),
        textField.topAnchor.constraint(equalTo: cell.contentView.topAnchor),
        textField.bottomAnchor.constraint(equalTo: cell.contentView.bottomAnchor),
      ])
    }
    return cell
  }
}

private extension UIColor {
  convenience init(argb: UInt32) {
    self.init(
      red: CGFloat((argb >> 16) & 0xff) / 255,
      green: CGFloat((argb >> 8) & 0xff) / 255,
      blue: CGFloat(argb & 0xff) / 255,
      alpha: CGFloat((argb >> 24) & 0xff) / 255
    )
  }
}
