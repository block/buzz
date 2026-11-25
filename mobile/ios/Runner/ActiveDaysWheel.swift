import Flutter
import UIKit

final class ActiveDaysWheelFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger

  init(messenger: FlutterBinaryMessenger) {
    self.messenger = messenger
  }

  func createArgsCodec() -> FlutterMessageCodec & NSObjectProtocol {
    FlutterStandardMessageCodec.sharedInstance()
  }

  func create(withFrame frame: CGRect, viewIdentifier id: Int64, arguments args: Any?) -> FlutterPlatformView {
    ActiveDaysWheel(frame: frame, id: id, messenger: messenger, options: args as? [String: Any] ?? [:])
  }
}

private final class ActiveDaysWheel: NSObject, FlutterPlatformView, UIPickerViewDataSource, UIPickerViewDelegate {
  private let picker: UIPickerView
  private let channel: FlutterMethodChannel
  private let maximum: Int

  init(frame: CGRect, id: Int64, messenger: FlutterBinaryMessenger, options: [String: Any]) {
    picker = UIPickerView(frame: frame)
    maximum = max(1, options["maximum"] as? Int ?? 365)
    channel = FlutterMethodChannel(name: "buzz/active_days_wheel/\(id)", binaryMessenger: messenger)
    super.init()
    picker.dataSource = self
    picker.delegate = self
    picker.overrideUserInterfaceStyle = options["dark"] as? Bool == true ? .dark : .light
    picker.selectRow(min(maximum, max(1, options["initial"] as? Int ?? 1)) - 1, inComponent: 0, animated: false)
    channel.setMethodCallHandler { [weak self] call, result in
      guard call.method == "selection" else { result(FlutterMethodNotImplemented); return }
      result(self.map { $0.picker.selectedRow(inComponent: 0) + 1 })
    }
  }

  deinit { channel.setMethodCallHandler(nil) }
  func view() -> UIView { picker }
  func numberOfComponents(in pickerView: UIPickerView) -> Int { 1 }
  func pickerView(_ pickerView: UIPickerView, numberOfRowsInComponent component: Int) -> Int { maximum }
  func pickerView(_ pickerView: UIPickerView, titleForRow row: Int, forComponent component: Int) -> String? {
    row == 0 ? "1 day" : "\(row + 1) days"
  }
}
