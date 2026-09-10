import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:google_sign_in_platform_interface/google_sign_in_platform_interface.dart';
import '../test/support/google_controls_scenario.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  var converted = false;
  final completed = <String>[];
  setUpAll(() async {
    if (Platform.isAndroid) {
      // Read the preview package's application configuration through the real
      // native plugin. No Google account request or consent UI is initiated.
      final sdk = GoogleSignInPlatform.instance;
      await sdk.init(const InitParameters());
      expect(sdk.supportsAuthenticate(), true);
    }
  });
  setUp(() => converted = false);
  Future<void> capture(WidgetTester tester, String name) async {
    if (Platform.isAndroid && !converted) {
      await binding.convertFlutterSurfaceToImage();
      converted = true;
    }
    await tester.pumpAndSettle();
    await binding.takeScreenshot(name);
  }

  testWidgets('native Google permission, cancellation and cleanup controls', (
    tester,
  ) async {
    await googleConsentControls(
      tester,
      capture: (name) => capture(tester, name),
    );
    completed.add('consent-cancel-retry-cleanup');
    binding.reportData = {...?binding.reportData, 'google_controls': completed};
  });
  testWidgets('native browsing and changed choices while Google is pending', (
    tester,
  ) async {
    await googlePendingControls(
      tester,
      capture: (name) => capture(tester, name),
    );
    completed.add('pending-browse-changed-choices');
    binding.reportData = {...?binding.reportData, 'google_controls': completed};
  });
}
