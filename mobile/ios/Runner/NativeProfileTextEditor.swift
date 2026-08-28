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
    guard call.method == "present" else {
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

    pendingResult = result
    presentedController = navigationController
    presenter.present(navigationController, animated: true) { [weak self] in
      navigationController.presentationController?.delegate = self
    }
  }

  @MainActor
  private func finish(value: String?) {
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
  private func resolve(value: String?) {
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
  private let onSet: (String) -> Void

  private lazy var textField: UITextField = {
    let field = UITextField()
    field.translatesAutoresizingMaskIntoConstraints = false
    field.placeholder = placeholder
    field.text = initialValue
    field.font = .preferredFont(forTextStyle: .body)
    field.adjustsFontForContentSizeCategory = true
    field.clearButtonMode = .whileEditing
    field.returnKeyType = .done
    field.autocapitalizationType = .sentences
    field.delegate = self
    field.addTarget(self, action: #selector(textDidChange), for: .editingChanged)
    return field
  }()

  private lazy var textView: UITextView = {
    let view = UITextView()
    view.translatesAutoresizingMaskIntoConstraints = false
    view.text = initialValue
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
    label.text = placeholder
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
    onCancel: @escaping () -> Void,
    onSet: @escaping (String) -> Void
  ) {
    self.initialValue = initialValue
    self.placeholder = placeholder
    self.multiline = multiline
    self.pageBackgroundColor = pageBackgroundColor
    self.containerBackgroundColor = containerBackgroundColor
    self.containerCornerRadius = containerCornerRadius
    self.allowUnchangedSubmission = allowUnchangedSubmission
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
      title: "Set",
      style: .done,
      target: self,
      action: #selector(setTapped)
    )
    updateNavigation()
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
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
    allowUnchangedSubmission
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
    navigationItem.rightBarButtonItem?.isEnabled = hasUnsavedChanges
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

  @objc private func setTapped() {
    guard hasUnsavedChanges else { return }
    onSet(currentValue)
  }

  func textFieldShouldReturn(_ textField: UITextField) -> Bool {
    setTapped()
    return false
  }

  override func numberOfSections(in tableView: UITableView) -> Int { 1 }

  override func tableView(
    _ tableView: UITableView,
    numberOfRowsInSection section: Int
  ) -> Int { 1 }

  override func tableView(
    _ tableView: UITableView,
    heightForRowAt indexPath: IndexPath
  ) -> CGFloat {
    multiline ? 132 : 52
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
    if multiline {
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
