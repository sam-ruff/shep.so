import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'support/bulk_controls_scenario.dart';

void main() {
  for (final dark in [false, true]) {
    testWidgets(
      'selection, review, progress, Undo and History controls (${dark ? 'dark' : 'light'})',
      (tester) async {
        tester.view.physicalSize = const Size(1080, 2280);
        tester.view.devicePixelRatio = 2.625;
        addTearDown(tester.view.reset);
        await bulkControlsScenario(tester, dark: dark);
      },
    );
  }
}
