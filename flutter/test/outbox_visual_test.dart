import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/outbox.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'mail_activity_visual_test.dart' show loadPreviewFonts, VisualSettings;
import 'outbox_test.dart' show ControlledOutbox;

void main() {
  for (final brightness in Brightness.values) {
    testWidgets('compact queued Outbox ${brightness.name}', (tester) async {
      await loadPreviewFonts();
      tester.view.physicalSize = const Size(390, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final repository = ControlledOutbox(state: 'waiting')
        ..outgoingError = 'Reconnect Personal to resume delivery.';
      final workspace = Workspace(repository, VisualSettings());
      addTearDown(workspace.dispose);
      await tester.pumpWidget(
        MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: shepTheme(brightness),
          home: OutboxScreen(workspace: workspace),
        ),
      );
      await tester.pump();
      await tester.pump();
      expect(find.text('Resume delivery'), findsOneWidget);
      expect(find.text('Cancel'), findsOneWidget);
      expect(find.text('Return to drafts'), findsOneWidget);
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/outbox_waiting_${brightness.name}.png'),
      );
    });
  }
}
