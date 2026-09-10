import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:webview_flutter/webview_flutter.dart';
import 'package:webview_flutter_android/webview_flutter_android.dart';
import 'package:webview_flutter_wkwebview/webview_flutter_wkwebview.dart';

Map<String, dynamic> _decode(String value) =>
    (jsonDecode(value) as Map).cast<String, dynamic>();
String _encode(Map<String, Object?> value) => jsonEncode(value);

class FormattedView extends StatefulWidget {
  const FormattedView({
    super.key,
    required this.document,
    required this.commands,
    required this.onMessage,
    required this.onError,
  });
  final String document;
  final Stream<Map<String, Object?>> commands;
  final void Function(Map<String, dynamic>) onMessage;
  final VoidCallback onError;
  @override
  State<FormattedView> createState() => _FormattedViewState();
}

class _FormattedViewState extends State<FormattedView> {
  late final WebViewController controller;
  StreamSubscription<Map<String, Object?>>? subscription;
  final pending = <String, Map<String, Object?>>{};
  bool closed = false, sending = false, initialized = false;
  Timer? startup;
  @override
  void initState() {
    super.initState();
    controller = WebViewController(
      onPermissionRequest: (request) => request.deny(),
    );
    subscription = widget.commands.listen((command) {
      pending[command['type'] as String] = command;
      unawaited(send());
    });
    unawaited(initialize());
  }

  Future<void> initialize() async {
    try {
      await controller.setNavigationDelegate(
        NavigationDelegate(
          onNavigationRequest: (_) => NavigationDecision.prevent,
          onWebResourceError: (error) {
            if (error.isForMainFrame == true && !closed) widget.onError();
          },
        ),
      );
      if (controller.platform case final AndroidWebViewController android) {
        await android.setAllowFileAccess(false);
        await android.setAllowContentAccess(false);
      }
      if (controller.platform case final WebKitWebViewController apple) {
        await apple.setAllowsBackForwardNavigationGestures(false);
        await apple.setAllowsLinkPreview(false);
      }
      await controller.setJavaScriptMode(JavaScriptMode.unrestricted);
      await controller.addJavaScriptChannel(
        'ShepReader',
        onMessageReceived: (message) async {
          if (closed) return;
          try {
            final value = await compute(_decode, message.message);
            if (closed) return;
            if (value['type'] == 'ready') startup?.cancel();
            widget.onMessage(value);
          } catch (_) {
            if (!closed) widget.onError();
          }
        },
      );
      if (closed) return;
      initialized = true;
      startup = Timer(const Duration(seconds: 10), () {
        if (!closed) widget.onError();
      });
      // This owned .invalid origin is never fetched. CSP allows only the fixed
      // runtime and converted blobs; navigation and device content are denied.
      await controller.loadHtmlString(
        widget.document,
        baseUrl: 'https://shep-reader.invalid/',
      );
      if (!closed) setState(() {});
    } catch (_) {
      if (!closed) widget.onError();
    }
  }

  Future<void> send() async {
    if (closed || !initialized || sending) return;
    sending = true;
    try {
      while (pending.isNotEmpty && !closed) {
        final key = pending.keys.first,
            value = pending.remove(pending.keys.first)!;
        final encoded = await compute(_encode, value);
        // A newer command of this kind supersedes one still being encoded.
        if (closed || pending.containsKey(key)) continue;
        await controller.runJavaScript('window.shepReaderCommand?.($encoded)');
      }
    } catch (_) {
      if (!closed) widget.onError();
    } finally {
      sending = false;
    }
  }

  @override
  void dispose() {
    closed = true;
    startup?.cancel();
    pending.clear();
    unawaited(subscription?.cancel());
    unawaited(disposeDocument());
    super.dispose();
  }

  Future<void> disposeDocument() async {
    try {
      await controller.runJavaScript('window.shepReaderDispose?.()');
      await controller.removeJavaScriptChannel('ShepReader');
    } catch (_) {
      /* The platform view may already have been released. */
    }
  }

  @override
  Widget build(BuildContext context) {
    var parameters = PlatformWebViewWidgetCreationParams(
      controller: controller.platform,
      gestureRecognizers: {
        Factory<EagerGestureRecognizer>(() => EagerGestureRecognizer()),
      },
    );
    if (controller.platform is AndroidWebViewController) {
      parameters =
          AndroidWebViewWidgetCreationParams.fromPlatformWebViewWidgetCreationParams(
            parameters,
            displayWithHybridComposition: true,
          );
    }
    return Semantics(
      container: true,
      explicitChildNodes: true,
      label: 'Formatted message',
      child: WebViewWidget.fromPlatformCreationParams(params: parameters),
    );
  }
}
