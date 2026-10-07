import 'dart:async';
import 'package:flutter/services.dart';

/// `mailto:` links the operating system asked Shep to open. The platform holds
/// them until Dart takes them, so a link that starts Shep is not lost while the
/// device cache opens.
abstract interface class MailtoLinks {
  /// Fires when the platform holds links that have not been taken.
  Stream<void> get arrivals;

  /// Links received since the last call, oldest first.
  Future<List<String>> take();
}

class PlatformMailtoLinks implements MailtoLinks {
  PlatformMailtoLinks() {
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'available') _arrivals.add(null);
    });
  }
  static const _channel = MethodChannel('so.shep/mailto');
  final _arrivals = StreamController<void>.broadcast();
  @override
  Stream<void> get arrivals => _arrivals.stream;
  @override
  Future<List<String>> take() async {
    try {
      return await _channel.invokeListMethod<String>('take') ?? const [];
    } on MissingPluginException {
      return const [];
    }
  }
}
