import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import '../test/support/profile_creation_controls.dart';

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

  testWidgets(
    'native profile publication review, retry, pagination and appearance',
    (tester) async {
      await profileCreationReviewControls(
        tester,
        capture: (name) => capture(tester, name),
      );
      completed.add('creation-review-retry-pages-appearance');
      binding.reportData = {
        ...?binding.reportData,
        'profile_creation': completed,
      };
    },
  );
  testWidgets(
    'native profile publication pause and independent mail navigation',
    (tester) async {
      await profileCreationPauseControls(
        tester,
        capture: (name) => capture(tester, name),
      );
      completed.add('creation-pause-browse-resume');
      binding.reportData = {
        ...?binding.reportData,
        'profile_creation': completed,
      };
    },
  );
}
