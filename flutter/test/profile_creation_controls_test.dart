import 'package:flutter_test/flutter_test.dart';
import 'support/profile_creation_controls.dart';

void main() {
  testWidgets(
    'publication review, paging, settings, retry and appearance use actual controls',
    profileCreationReviewControls,
  );
  testWidgets(
    'publication pause keeps mail browsing available and resumes the same profile',
    profileCreationPauseControls,
  );
}
