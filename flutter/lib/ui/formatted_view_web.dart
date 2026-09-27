import 'dart:async';
import 'dart:js_interop';
import 'package:flutter/material.dart';
import 'package:web/web.dart' as web;

class FormattedView extends StatefulWidget {
  const FormattedView({
    super.key,
    required this.document,
    required this.commands,
    required this.onMessage,
    required this.onError,
    this.background,
  });
  final String document;

  /// The document's reported canvas, painted by the frame itself.
  final Color? background;
  final Stream<Map<String, Object?>> commands;
  final void Function(Map<String, dynamic>) onMessage;
  final VoidCallback onError;
  @override
  State<FormattedView> createState() => _FormattedViewState();
}

class _FormattedViewState extends State<FormattedView> {
  web.HTMLIFrameElement? frame;
  StreamSubscription<Map<String, Object?>>? subscription;
  late final JSFunction receive;
  Timer? startup;
  @override
  void initState() {
    super.initState();
    receive = ((web.MessageEvent event) {
      if (!mounted ||
          frame == null ||
          !(event.source?.strictEquals(frame!.contentWindow).toDart ?? false)) {
        return;
      }
      final value = event.data.dartify();
      if (value is! Map) return;
      if (value['type'] == 'ready') startup?.cancel();
      widget.onMessage(value.cast<String, dynamic>());
    }).toJS;
    web.window.addEventListener('message', receive);
    subscription = widget.commands.listen(
      (value) => frame?.contentWindow?.postMessage(value.jsify(), '*'.toJS),
    );
  }

  @override
  void didUpdateWidget(FormattedView old) {
    super.didUpdateWidget(old);
    final background = widget.background;
    if (background != null && background != old.background) {
      final rgb = background.toARGB32() & 0xffffff;
      frame?.style.backgroundColor =
          '#${rgb.toRadixString(16).padLeft(6, '0')}';
    }
  }

  @override
  Widget build(BuildContext context) => Semantics(
    container: true,
    explicitChildNodes: true,
    child: HtmlElementView.fromTagName(
      tagName: 'iframe',
      onElementCreated: (element) {
        final node = frame = element as web.HTMLIFrameElement;
        node.title = 'Formatted message';
        node.setAttribute('sandbox', 'allow-scripts');
        node.referrerPolicy = 'no-referrer';
        node.style
          ..width = '100%'
          ..height = '100%'
          ..border = '0';
        node.srcdoc = widget.document.toJS;
        startup = Timer(const Duration(seconds: 10), () {
          if (mounted) widget.onError();
        });
      },
    ),
  );
  @override
  void dispose() {
    startup?.cancel();
    unawaited(subscription?.cancel());
    web.window.removeEventListener('message', receive);
    frame = null;
    super.dispose();
  }
}
