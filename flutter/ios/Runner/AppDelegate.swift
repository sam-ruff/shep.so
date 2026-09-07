import Flutter
import UIKit
import WebKit

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
    if let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "ShepMessagePrint") {
      MessagePrintPlugin.register(with: registrar)
    }
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

/// Prints the fully prepared document with UIKit's native printer/PDF UI.
/// WebKit uses ephemeral storage and only the fixed CSP-hashed print runtime.
private final class MessagePrintPlugin: NSObject, FlutterPlugin, WKNavigationDelegate, WKScriptMessageHandler {
  private let registrar: FlutterPluginRegistrar
  private var pending: FlutterResult?
  private var web: WKWebView?
  private var generation: String?
  private var presenting = false
  private var title = "Shep message"
  private var deadline: Timer?
  init(registrar: FlutterPluginRegistrar) { self.registrar = registrar }
  static func register(with registrar: FlutterPluginRegistrar) {
    let channel = FlutterMethodChannel(name: "so.shep/message-print", binaryMessenger: registrar.messenger())
    registrar.addMethodCallDelegate(MessagePrintPlugin(registrar: registrar), channel: channel)
  }
  func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    guard call.method == "print" else { result(FlutterMethodNotImplemented); return }
    guard pending == nil else { result(FlutterError(code: "busy", message: "Finish the current print dialog first.", details: nil)); return }
    guard let args = call.arguments as? [String: Any], let html = args["document"] as? String,
          !html.isEmpty, html.utf8.count <= 96 * 1024 * 1024,
          let token = args["generation"] as? String, !token.isEmpty, token.utf8.count <= 128 else {
      result(FlutterError(code: "invalid", message: "Reopen this message and retry printing.", details: nil)); return
    }
    guard UIPrintInteractionController.isPrintingAvailable else {
      result(FlutterError(code: "unavailable", message: "Printing is unavailable on this device.", details: nil)); return
    }
    pending = result; generation = token
    title = String((args["title"] as? String ?? "Shep message").unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) }.map { String($0) }.joined().prefix(200))
    let configuration = WKWebViewConfiguration()
    configuration.websiteDataStore = .nonPersistent()
    configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
    configuration.userContentController.add(self, name: "ShepPrint")
    let view = WKWebView(frame: CGRect(x: 0, y: 0, width: 595, height: 842), configuration: configuration)
    view.navigationDelegate = self
    web = view
    deadline = Timer.scheduledTimer(withTimeInterval: 30, repeats: false) { [weak self] _ in
      self?.fail("The print document did not finish preparing. Try plain text or retry.")
    }
    view.loadHTMLString(html, baseURL: nil)
  }
  func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
    guard message.webView === web, message.frameInfo.isMainFrame, pending != nil,
          let value = message.body as? [String: Any], value["generation"] as? String == generation else { return }
    if value["type"] as? String == "error" { fail("Could not format the print document. Try plain text or retry.") }
    else if value["type"] as? String == "ready" { openDialog() }
  }
  private func openDialog() {
    guard !presenting else { return }
    guard let web = web, let presenter = registrar.viewController else { fail("Could not open the print dialog. Retry."); return }
    deadline?.invalidate(); deadline = nil
    presenting = true
    let controller = UIPrintInteractionController.shared
    let info = UIPrintInfo(dictionary: nil)
    info.outputType = .general; info.jobName = title
    controller.printInfo = info
    controller.printFormatter = web.viewPrintFormatter()
    let completion: (UIPrintInteractionController, Bool, Error?) -> Void = { [weak self] _, _, error in
      if error != nil { self?.fail("Printing did not finish. Check the printer or retry.") }
      else { self?.finish(nil) } // Cancellation is not described as printed mail.
    }
    let opened: Bool
    if UIDevice.current.userInterfaceIdiom == .pad {
      opened = controller.present(from: CGRect(x: presenter.view.bounds.midX, y: 0, width: 1, height: 1), in: presenter.view, animated: true, completionHandler: completion)
    } else { opened = controller.present(animated: true, completionHandler: completion) }
    if !opened { fail("Could not open the print dialog. Retry.") }
  }
  func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
    decisionHandler(navigationAction.request.url?.scheme == "about" && navigationAction.navigationType == .other ? .allow : .cancel)
  }
  func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) { fail("Could not load the print document. Retry.") }
  func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) { fail("Could not load the print document. Retry.") }
  func webViewWebContentProcessDidTerminate(_ webView: WKWebView) { fail("The print renderer stopped. Try plain text or retry.") }
  private func fail(_ message: String) { finish(FlutterError(code: "print", message: message, details: nil)) }
  private func finish(_ value: Any?) {
    let result = pending; pending = nil; generation = nil; presenting = false
    deadline?.invalidate(); deadline = nil
    web?.navigationDelegate = nil
    web?.configuration.userContentController.removeScriptMessageHandler(forName: "ShepPrint")
    web?.stopLoading(); web = nil
    result?(value)
  }
  func detachFromEngine(for registrar: FlutterPluginRegistrar) { finish(FlutterError(code: "closed", message: "Printing was interrupted when Shep closed. Reopen the message and retry.", details: nil)) }
}
