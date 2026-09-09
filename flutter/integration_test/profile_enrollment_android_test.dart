import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import '../test/support/profile_enrollment_controls.dart';

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

  testWidgets('native enrollment review, application retry and reconnect', (
    tester,
  ) async {
    await profileEnrollmentReviewControls(
      tester,
      capture: (name) => capture(tester, name),
    );
    completed.add('enrollment-review-retry-reconnect');
    binding.reportData = {
      ...?binding.reportData,
      'profile_enrollment': completed,
    };
  });
  testWidgets('native enrollment pause, mail browsing and resume', (
    tester,
  ) async {
    await profileEnrollmentPauseControls(
      tester,
      capture: (name) => capture(tester, name),
    );
    completed.add('enrollment-pause-browse-resume');
    binding.reportData = {
      ...?binding.reportData,
      'profile_enrollment': completed,
    };
  });
}
