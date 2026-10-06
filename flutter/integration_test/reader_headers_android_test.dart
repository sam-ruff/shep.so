import 'dart:io';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import '../test/support/reader_headers_scenario.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  var converted = false;
  final completed = <String>[];
  setUp(() => converted = false);
  for (final dark in [false, true]) {
    final scheme = dark ? 'dark' : 'light';
    testWidgets('reader header clipboard and refresh controls ($scheme)', (
      tester,
    ) async {
      await readerHeadersScenario(
        tester,
        dark: dark,
        clipboard: () async => (await Clipboard.getData('text/plain'))?.text,
        capture: (name) async {
          if (Platform.isAndroid && !converted) {
            await binding.convertFlutterSurfaceToImage();
            converted = true;
          }
          await tester.pump(const Duration(milliseconds: 100));
          await binding.takeScreenshot(name);
        },
      );
      completed.add('reader-headers-clipboard-refresh-$scheme');
      binding.reportData = {...?binding.reportData, 'scenarios': completed};
    });
  }
}
