import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/formatted_message.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/remote_images.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'package:webview_flutter_platform_interface/webview_flutter_platform_interface.dart';
import 'support/fake_webview.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

final keyA = 'a' * 64, keyB = 'b' * 64;

/// Synthetic preparation and image service; actions still use real controls.
class ImageRepository extends PreviewRepository
    implements FormattedMessageRepository, RemoteImageRepository {
  ImageRepository() : super(delay: Duration.zero);
  final requests =
      <
        ({
          String id,
          List<String> keys,
          ImageRules rules,
          Completer<RemoteImageBatch> reply,
        })
      >[];
  int forgotten = 0;
  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) async => PreparedMessage(
    signature: id,
    text: 'Plain fallback',
    document: '<!doctype html><html data-generation="$generation"></html>',
    senderAddress: 'news@example.test',
    senderDomain: 'example.test',
    remoteImages: [
      RemoteImage('https://images.example.test/a.png', 'A', key: keyA),
      RemoteImage('https://images.example.test/b.png', 'B', key: keyB),
    ],
  );
  @override
  Future<RemoteImageBatch> remoteImages(
    String id, {
    required List<String> keys,
    required ImageRules rules,
  }) {
    final reply = Completer<RemoteImageBatch>();
    requests.add((id: id, keys: keys, rules: rules, reply: reply));
    return reply.future;
  }

  @override
  Future<void> forgetRemoteImages() async => forgotten++;
}

RemoteImageBatch batch(List<String> keys, {Map<String, String>? failed}) =>
    RemoteImageBatch(
      images: {
        for (final key in keys) key: const RemoteImageBytes('UklG', 4, 2),
      },
      failed: failed ?? const {},
    );

/// The WebView bridge encodes on a worker isolate, which needs real time.
Future<void> bridge(WidgetTester tester, [void Function()? action]) async {
  await tester.runAsync(() async {
    action?.call();
    for (var i = 0; i < 6; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
  });
  await tester.pump();
  await tester.pump();
}

/// Compact phone surface with the bundled fonts, as the other goldens use.
Future<void> compact(WidgetTester tester) async {
  await tester.binding.setSurfaceSize(const Size(390, 760));
  addTearDown(() => tester.binding.setSurfaceSize(null));
  await (FontLoader(
    'Roboto',
  )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
  await (FontLoader('NotoSans')
        ..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))
        ..addFont(rootBundle.load('assets/NotoSans-SemiBold.ttf')))
      .load();
}

Future<void> readerScenario(WidgetTester tester, {required bool dark}) async {
  await compact(tester);
  final platform = FakeWebViewPlatform();
  WebViewPlatform.instance = platform;
  final repo = ImageRepository();
  final settings = MemorySettings()
    ..value = Preferences(appearance: dark ? ThemeMode.dark : ThemeMode.light);
  final workspace = Workspace(repo, settings);
  await workspace.initialize();
  workspace.setForeground(false);
  await tester.pumpWidget(ShepApp(workspace: workspace));
  await tester.pumpAndSettle();
  await tester.tap(find.text('A little room for good ideas'));
  for (var i = 0; i < 8; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
  await bridge(tester);
  final first = platform.controllers.single;
  expect(first.baseUrl, 'https://shep-reader.invalid/');
  await bridge(tester, () => first.post({'type': 'ready'}));
  expect(find.text('2 remote images blocked.'), findsOneWidget);
  expect(repo.requests, isEmpty, reason: 'Blocked by default');

  await tester.tap(find.byKey(const ValueKey('images-load')));
  await tester.pump();
  await tester.pump();
  expect(repo.requests.single.keys, [keyA, keyB]);
  expect(repo.requests.single.rules.trust.messages, ['1']);
  expect(settings.value.imageTrust.messages, ['1']);
  expect(find.text('Loading 2 remote images'), findsOneWidget);
  repo.requests.single.reply.complete(
    batch(
      [keyA],
      failed: {keyB: 'The image server did not return this image.'},
    ),
  );
  await bridge(tester);
  final delivered = first.commandsOf('images').single;
  expect(delivered['generation'], first.generation);
  expect((delivered['images'] as Map).keys, [keyA]);
  expect(find.text('1 of 2 remote images could not load.'), findsOneWidget);
  await expectLater(
    find.byType(ShepApp),
    matchesGoldenFile(
      'goldens/remote_images_reader_${dark ? 'dark' : 'light'}.png',
    ),
  );
  await bridge(
    tester,
    () => first.post({
      'type': 'images',
      'keys': [keyA],
    }),
  );

  await tester.tap(find.byKey(const ValueKey('images-retry')));
  await tester.pump();
  expect(repo.requests.last.keys, [keyB]);
  repo.requests.last.reply.complete(batch([keyB]));
  await bridge(tester);
  expect(first.commandsOf('images'), hasLength(2));
  expect(find.text('2 remote images allowed.'), findsOneWidget);
  // Arrivals and retry reach the same document; nothing reloaded it.
  expect(platform.controllers, hasLength(1));
  expect(first.disposed, isFalse);

  await tester.tap(find.byKey(const ValueKey('images-block')));
  await tester.pump();
  await bridge(tester);
  expect(repo.forgotten, 1);
  expect(settings.value.imageTrust.messages, isEmpty);
  // Revocation replaces the document so no permitted pixels remain.
  expect(platform.controllers, hasLength(2));
  expect(first.disposed, isTrue);
  final second = platform.controllers.last;
  expect(second.generation, isNot(first.generation));
  expect(second.commandsOf('images'), isEmpty);
  await bridge(tester, () => second.post({'type': 'ready'}));
  expect(find.text('2 remote images blocked.'), findsOneWidget);

  await tester.tap(find.byKey(const ValueKey('images-more')));
  await tester.pumpAndSettle();
  await tester.tap(find.text('Always for news@example.test'));
  // The loading indicator animates until the held request completes.
  for (var i = 0; i < 4; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
  expect(repo.requests.last.rules.trust.senders, ['news@example.test']);
  repo.requests.last.reply.complete(batch([keyA, keyB]));
  await bridge(tester);
  expect(second.commandsOf('images'), hasLength(1));
  expect(settings.value.imageTrust.senders, ['news@example.test']);
  expect(find.text('2 remote images allowed.'), findsOneWidget);
  await tester.pumpWidget(const SizedBox());
  await tester.pump();
  workspace.dispose();
}

Future<void> preferencesScenario(
  WidgetTester tester, {
  required bool dark,
}) async {
  await compact(tester);
  final repo = ImageRepository();
  final settings = MemorySettings();
  final workspace = Workspace(repo, settings);
  await workspace.initialize();
  await workspace.allowImages(
    ImageGrant.sender,
    id: '1',
    address: 'news@example.test',
  );
  await workspace.allowImages(
    ImageGrant.domain,
    id: '1',
    domain: 'example.test',
  );
  await workspace.allowImages(ImageGrant.message, id: '2');
  await tester.pumpWidget(
    MaterialApp(
      theme: shepTheme(dark ? Brightness.dark : Brightness.light),
      home: Scaffold(body: PreferencesView(workspace: workspace)),
    ),
  );
  await tester.pumpAndSettle();
  final policy = find.byKey(const ValueKey('preference-images'));
  await tester.ensureVisible(policy);
  await tester.pumpAndSettle();
  expect(find.text('1 message, 1 sender, 1 domain'), findsOneWidget);
  await expectLater(
    find.byType(Scaffold),
    matchesGoldenFile(
      'goldens/remote_images_preferences_${dark ? 'dark' : 'light'}.png',
    ),
  );

  await tester.tap(
    find.descendant(of: policy, matching: find.text('Block all')),
  );
  await tester.pumpAndSettle();
  await tester.tap(find.text('Allow all').last);
  await tester.pumpAndSettle();
  expect(workspace.preferences.imagePolicy, ImagePolicy.allowAll);
  expect(settings.value.profileSettings()['image_policy'], 'AllowAll');
  expect(repo.forgotten, 0, reason: 'Widening keeps cached images');

  await tester.tap(
    find.descendant(of: policy, matching: find.text('Allow all')),
  );
  await tester.pumpAndSettle();
  await tester.tap(find.text('Contacts only').last);
  await tester.pumpAndSettle();
  expect(workspace.preferences.imagePolicy, ImagePolicy.contacts);
  expect(repo.forgotten, 1);

  await tester.ensureVisible(find.byTooltip('Remove news@example.test'));
  await tester.tap(find.byTooltip('Remove news@example.test'));
  await tester.pumpAndSettle();
  expect(workspace.preferences.imageTrust.senders, isEmpty);
  expect(repo.forgotten, 2);

  final contacts = find.byKey(const ValueKey('preference-contacts'));
  await tester.ensureVisible(contacts);
  await tester.enterText(contacts, 'Alex <alex@example.com>');
  await tester.tap(find.byKey(const ValueKey('preference-contacts-add')));
  await tester.pumpAndSettle();
  expect(find.textContaining('Enter email addresses'), findsOneWidget);
  expect(workspace.preferences.imageTrust.contacts, isEmpty);
  await tester.enterText(contacts, 'alex@example.com, Maya@Example.com');
  await tester.tap(find.byKey(const ValueKey('preference-contacts-add')));
  await tester.pumpAndSettle();
  expect(find.textContaining('Enter email addresses'), findsNothing);
  expect(settings.value.imageTrust.contacts, [
    'alex@example.com',
    'maya@example.com',
  ]);
  expect(find.byTooltip('Remove maya@example.com'), findsOneWidget);
  await tester.tap(find.byTooltip('Remove maya@example.com'));
  await tester.pumpAndSettle();
  expect(workspace.preferences.imageTrust.contacts, ['alex@example.com']);
  expect(repo.forgotten, 3, reason: 'A removed contact narrows Contacts only');

  final clear = find.byKey(const ValueKey('preference-image-exceptions-clear'));
  await tester.ensureVisible(clear);
  await tester.tap(clear);
  await tester.pumpAndSettle();
  expect(workspace.preferences.imageTrust.hasExceptions, isFalse);
  expect(workspace.preferences.imageTrust.contacts, ['alex@example.com']);
  expect(find.text('0 messages, 0 senders, 0 domains'), findsOneWidget);
  workspace.dispose();
}

void main() {
  for (final dark in [false, true]) {
    final theme = dark ? 'dark' : 'light';
    testWidgets(
      'reader loads permitted images into the same document and revocation replaces it ($theme)',
      (tester) => readerScenario(tester, dark: dark),
    );
    testWidgets(
      'preferences change the synced policy and local exceptions through real controls ($theme)',
      (tester) => preferencesScenario(tester, dark: dark),
    );
  }
}
