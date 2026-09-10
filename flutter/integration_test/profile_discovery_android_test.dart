import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import '../test/support/profile_discovery_controls.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  var converted = false;
  final completed = <String>[];
  setUp(() => converted = false);
  Future<void> capture(WidgetTester tester, String name) async {
    if (Platform.isAndroid && !converted) {
      await binding.convertFlutterSurfaceToImage();
      converted = true;
    }
    await tester.pumpAndSettle();
    await binding.takeScreenshot(name);
  }

  testWidgets('native profile discovery retry, pagination and appearance', (
    tester,
  ) async {
    await profileDiscoveryControls(
      tester,
      capture: (name) => capture(tester, name),
    );
    completed.add('discovery-retry-pages-appearance');
    binding.reportData = {
      ...?binding.reportData,
      'profile_discovery': completed,
    };
  });
  testWidgets(
    'native profile discovery pause and independent mail navigation',
    (tester) async {
      await profileDiscoveryPendingControls(
        tester,
        capture: (name) => capture(tester, name),
      );
      completed.add('discovery-pause-browse-resume');
      binding.reportData = {
        ...?binding.reportData,
        'profile_discovery': completed,
      };
    },
  );
}
