import 'package:flutter_test/flutter_test.dart';
import 'support/profile_enrollment_controls.dart';

void main() {
  testWidgets(
    'review pages, connection details, apply retry and reconnect use actual controls',
    profileEnrollmentReviewControls,
  );
  testWidgets(
    'pause enrollment, browse mail and resume through saved controls',
    profileEnrollmentPauseControls,
  );
  testWidgets('review footer clears the device navigation inset', (
    tester,
  ) async {
    tester.view.padding = const FakeViewPadding(bottom: 180);
    addTearDown(tester.view.resetPadding);
    await profileEnrollmentReviewControls(tester);
  });
}
