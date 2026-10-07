import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:webview_flutter_platform_interface/webview_flutter_platform_interface.dart';

/// Host-test WebView. It renders nothing, records what the reader sends and
/// lets a test answer as the confined runtime would through its bridge.
class FakeWebViewPlatform extends WebViewPlatform {
  final controllers = <FakeWebViewController>[];
  @override
  PlatformWebViewController createPlatformWebViewController(
    PlatformWebViewControllerCreationParams params,
  ) {
    final controller = FakeWebViewController(params);
    controllers.add(controller);
    return controller;
  }

  @override
  PlatformNavigationDelegate createPlatformNavigationDelegate(
    PlatformNavigationDelegateCreationParams params,
  ) => _Delegate(params);

  @override
  PlatformWebViewWidget createPlatformWebViewWidget(
    PlatformWebViewWidgetCreationParams params,
  ) => _Widget(params);
}

class FakeWebViewController extends PlatformWebViewController {
  FakeWebViewController(super.params) : super.implementation();
  String? html, baseUrl;
  final scripts = <String>[];
  JavaScriptChannelParams? channel;
  static const _prefix = 'window.shepReaderCommand?.(';

  /// Runtime commands, decoded from the scripts the reader ran.
  List<Map<String, dynamic>> get commands => [
    for (final script in scripts)
      if (script.startsWith(_prefix))
        (jsonDecode(script.substring(_prefix.length, script.length - 1)) as Map)
            .cast<String, dynamic>(),
  ];
  List<Map<String, dynamic>> commandsOf(String type) =>
      commands.where((command) => command['type'] == type).toList();
  bool get disposed => scripts.contains('window.shepReaderDispose?.()');

  /// The test documents carry their generation in a data attribute.
  String get generation =>
      RegExp(r'data-generation="([^"]+)"').firstMatch(html ?? '')!.group(1)!;

  void post(Map<String, Object?> message) => channel!.onMessageReceived(
    JavaScriptMessage(
      message: jsonEncode({'generation': generation, ...message}),
    ),
  );

  @override
  Future<void> loadHtmlString(String html, {String? baseUrl}) async {
    this.html = html;
    this.baseUrl = baseUrl;
  }

  @override
  Future<void> runJavaScript(String javaScript) async =>
      scripts.add(javaScript);
  @override
  Future<void> addJavaScriptChannel(JavaScriptChannelParams params) async =>
      channel = params;
  @override
  Future<void> removeJavaScriptChannel(String name) async {}
  @override
  Future<void> setJavaScriptMode(JavaScriptMode javaScriptMode) async {}
  @override
  Future<void> setPlatformNavigationDelegate(
    PlatformNavigationDelegate handler,
  ) async {}
  @override
  Future<void> setBackgroundColor(Color color) async {}
  @override
  Future<void> setOnPlatformPermissionRequest(
    void Function(PlatformWebViewPermissionRequest request) onPermissionRequest,
  ) async {}
}

class _Delegate extends PlatformNavigationDelegate {
  _Delegate(super.params) : super.implementation();
  @override
  Future<void> setOnNavigationRequest(
    NavigationRequestCallback onNavigationRequest,
  ) async {}
  @override
  Future<void> setOnWebResourceError(
    WebResourceErrorCallback onWebResourceError,
  ) async {}
}

class _Widget extends PlatformWebViewWidget {
  _Widget(super.params) : super.implementation();
  @override
  Widget build(BuildContext context) =>
      const SizedBox.expand(key: ValueKey('fake-webview'));
}
