import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    if let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "ShepAttachmentSave") {
      AttachmentSavePlugin.register(with: registrar)
    }
  }
}

/// The picker copies only the selected attachment to a user-chosen destination.
/// Temporary source bytes are protected and removed after success or cancellation.
private final class AttachmentSavePlugin: NSObject, FlutterPlugin, UIDocumentPickerDelegate {
  private let registrar: FlutterPluginRegistrar
  private var pending: FlutterResult?
  private var temporary: URL?
  init(registrar: FlutterPluginRegistrar) { self.registrar = registrar }
  static func register(with registrar: FlutterPluginRegistrar) {
    let channel = FlutterMethodChannel(name: "so.shep/attachment-save", binaryMessenger: registrar.messenger())
    registrar.addMethodCallDelegate(AttachmentSavePlugin(registrar: registrar), channel: channel)
  }
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    guard call.method == "save" else { result(FlutterMethodNotImplemented); return }
    guard pending == nil else { result(FlutterError(code: "busy", message: "Finish the current save first.", details: nil)); return }
    guard let args = call.arguments as? [String: Any], let name = args["name"] as? String,
          !name.isEmpty, !name.contains("/"), !name.contains("\\"), name != ".", name != "..",
          let bytes = args["bytes"] as? FlutterStandardTypedData, bytes.data.count <= 25 * 1024 * 1024 else {
      result(FlutterError(code: "invalid", message: "Reopen the attachment and retry.", details: nil)); return
    }
    pending = result
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("shep-attachment-\(UUID().uuidString)", isDirectory: true)
    temporary = directory
    let file = directory.appendingPathComponent(name)
    DispatchQueue.global(qos: .userInitiated).async {
      do {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false,
                                               attributes: [.protectionKey: FileProtectionType.complete])
        try bytes.data.write(to: file, options: [.atomic, .completeFileProtection])
        DispatchQueue.main.async {
          guard let presenter = self.registrar.viewController else {
            self.finish(FlutterError(code: "picker", message: "Could not open the save picker. Retry.", details: nil)); return
          }
          let picker = UIDocumentPickerViewController(forExporting: [file], asCopy: true)
          picker.delegate = self
          presenter.present(picker, animated: true)
        }
      } catch {
        DispatchQueue.main.async {
          self.finish(FlutterError(code: "write", message: "Could not prepare the attachment. Free space and retry.", details: nil))
        }
      }
    }
  }
  func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) { finish(false) }
  func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) { finish(!urls.isEmpty) }
  private func finish(_ value: Any) {
    let result = pending
    pending = nil
    if let directory = temporary {
      temporary = nil
      DispatchQueue.global(qos: .utility).async { try? FileManager.default.removeItem(at: directory) }
    }
    result?(value)
  }
}
