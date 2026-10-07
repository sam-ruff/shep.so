import 'dart:async';
import 'dart:math';
import 'package:flutter/foundation.dart';
import '../data/accounts.dart';
import '../data/formatted_message.dart';
import 'message_find.dart';
import 'remote_images.dart';

/// Per-reader lifetime. Results and runtime messages are bound to a generation;
/// preparing a new message never makes an old frame its action source.
class FormattedMessage extends ChangeNotifier {
  FormattedMessage(this.repository, this.id);
  final FormattedMessageRepository repository;
  final String id;
  String generation = '';
  PreparedMessage? prepared;
  String? error;
  bool loading = false, plain = false, ready = false, hasQuotes = false;
  List<String> blocks = [];

  /// Opaque ARGB background the document paints, once the runtime reports it.
  int? canvas;
  bool canvasDark = false;
  int layout = 0, _jump = -1;
  bool _disposed = false, _dark = false, _quotes = false;
  String _configuration = '', _highlight = '';
  final commands = StreamController<Map<String, Object?>>.broadcast(sync: true);
  bool get html => prepared?.document != null && error == null && !plain;

  /// Remote images delivered to this document, those its runtime has shown,
  /// and per-image failures with fixed native messages.
  final loadedImages = <String>{}, shownImages = <String>{};
  final imageErrors = <String, String>{};
  bool imagesLoading = false;
  String? imagesError;
  int _imageRun = 0, _batch = 0;
  final _undelivered = <Map<String, Object?>>[];

  /// The most images one message loads, in document order.
  static const maxRemoteImages = 64;
  RemoteImageRepository? get _images => repository is RemoteImageRepository
      ? repository as RemoteImageRepository
      : null;
  bool get supportsImages => _images != null;
  List<String> get _wantedImages => [
    for (final image
        in prepared?.remoteImages.take(maxRemoteImages) ??
            const <RemoteImage>[])
      if (image.key.isNotEmpty) image.key,
  ];

  /// Whether some permitted images have neither arrived nor failed.
  bool get imagesPending => _wantedImages.any(
    (key) => !loadedImages.contains(key) && !imageErrors.containsKey(key),
  );

  Future<void> load({required bool dark, required bool quotes}) async {
    final random = Random.secure();
    generation = List.generate(
      24,
      (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0'),
    ).join();
    final current = generation;
    _dark = dark;
    _quotes = quotes;
    loading = true;
    prepared = null;
    error = null;
    ready = false;
    layout = 0;
    blocks = [];
    hasQuotes = false;
    canvas = null;
    canvasDark = false;
    _configuration = '';
    _highlight = '';
    _jump = -1;
    _resetImages();
    notifyListeners();
    try {
      final result = await repository.formattedMessage(
        id,
        generation: current,
        dark: dark,
        quotes: quotes,
      );
      if (!_disposed && generation == current) prepared = result;
    } catch (_) {
      if (!_disposed && generation == current) {
        error =
            'Could not format this cached message. Use plain text or retry.';
      }
    } finally {
      if (!_disposed && generation == current) {
        loading = false;
        notifyListeners();
      }
    }
  }

  void setPlain(bool value) {
    plain = value;
    notifyListeners();
  }

  void _resetImages() {
    _imageRun++;
    loadedImages.clear();
    shownImages.clear();
    imageErrors.clear();
    imagesLoading = false;
    imagesError = null;
    _undelivered.clear();
  }

  /// Loads permitted images in small batches. Each batch reaches the existing
  /// document as a command, so position, selection and Find stay in place.
  /// A newer document, reload or [cancelImages] discards late results.
  Future<void> loadImages(ImageRules rules, {bool retry = false}) async {
    final images = _images;
    if (images == null || prepared == null || imagesLoading || _disposed) {
      return;
    }
    if (retry) {
      imageErrors.clear();
      imagesError = null;
    }
    final pending = [
      for (final key in _wantedImages)
        if (!loadedImages.contains(key) && !imageErrors.containsKey(key)) key,
    ];
    if (pending.isEmpty) return;
    final run = ++_imageRun, current = generation;
    bool stale() => _disposed || run != _imageRun || current != generation;
    imagesLoading = true;
    imagesError = null;
    notifyListeners();
    try {
      for (
        var start = 0;
        start < pending.length;
        start += RemoteImageRepository.batch
      ) {
        final batch = await images.remoteImages(
          id,
          keys: pending.sublist(
            start,
            min(start + RemoteImageRepository.batch, pending.length),
          ),
          rules: rules,
        );
        if (stale()) return;
        imageErrors.addAll(batch.failed);
        if (batch.images.isNotEmpty) {
          loadedImages.addAll(batch.images.keys);
          _deliver(batch.images);
        }
        notifyListeners();
      }
    } catch (e) {
      if (!stale()) {
        imagesError = e is MailOperationFailure
            ? e.message
            : 'Could not load images. Retry.';
      }
    } finally {
      if (!stale()) {
        imagesLoading = false;
        notifyListeners();
      }
    }
  }

  /// Stops issuing batches; a request already sent is ignored when it returns.
  void cancelImages() {
    _imageRun++;
    if (!imagesLoading) return;
    imagesLoading = false;
    notifyListeners();
  }

  void _deliver(Map<String, RemoteImageBytes> images) {
    final command = {
      'type': 'images',
      'generation': generation,
      'batch': ++_batch,
      'images': {
        for (final entry in images.entries) entry.key: entry.value.toJson(),
      },
    };
    if (ready) {
      commands.add(command);
    } else {
      _undelivered.add(command);
    }
  }

  void displayError(String expected) {
    if (_disposed || generation != expected) return;
    error = 'Could not display formatted mail. Use plain text or retry.';
    ready = false;
    notifyListeners();
  }

  bool receive(Map<String, dynamic> value) {
    if (_disposed || value['generation'] != generation) return false;
    switch (value['type']) {
      case 'ready':
        ready = true;
        _configuration = '';
        configure(dark: _dark, quotes: _quotes);
        for (final command in _undelivered) {
          commands.add(command);
        }
        _undelivered.clear();
        notifyListeners();
      case 'images':
        final keys = value['keys'];
        if (keys is! List || keys.any((key) => !loadedImages.contains(key))) {
          return false;
        }
        shownImages.addAll(keys.cast<String>());
        notifyListeners();
      case 'content':
        final revision = value['layout'], text = value['blocks'];
        if (revision is! int ||
            revision <= layout ||
            text is! List ||
            text.any((v) => v is! String)) {
          return false;
        }
        layout = revision;
        blocks = text.cast<String>();
        hasQuotes = value['hasQuotes'] == true;
        _highlight = '';
        notifyListeners();
      case 'canvas':
        final background = value['background'], scheme = value['scheme'];
        if (background is! String ||
            !RegExp(r'^#[0-9a-f]{6}$').hasMatch(background) ||
            (scheme != 'light' && scheme != 'dark')) {
          return false;
        }
        canvas = 0xff000000 | int.parse(background.substring(1), radix: 16);
        canvasDark = scheme == 'dark';
        notifyListeners();
      case 'error':
        displayError(generation);
    }
    return true;
  }

  void configure({required bool dark, required bool quotes}) {
    _dark = dark;
    _quotes = quotes;
    final key = '$dark:$quotes';
    if (!ready || _disposed || key == _configuration) return;
    _configuration = key;
    commands.add({
      'type': 'configure',
      'generation': generation,
      'dark': dark,
      'quotes': quotes,
      'shortcuts': ['Control+f', 'Meta+f', 'Escape'],
    });
  }

  /// Returns whether the outer Flutter scroll view should reveal the document.
  bool highlight(MessageFind find) {
    if (!ready || !html || layout == 0 || _disposed) return false;
    final key =
        '$layout:${find.revision}:${find.active}:${find.open}:${find.pending}';
    if (key == _highlight) return false;
    _highlight = key;
    final jump =
        find.open &&
        !find.pending &&
        find.hits.isNotEmpty &&
        find.jump != _jump;
    if (jump) _jump = find.jump;
    commands.add({
      'type': 'highlight',
      'generation': generation,
      'layout': layout,
      'revision': find.revision,
      'hits': find.open ? find.hits.map((h) => h.toJson()).toList() : [],
      'active': find.active,
      'jump': jump,
    });
    return jump;
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(commands.close());
    super.dispose();
  }
}
