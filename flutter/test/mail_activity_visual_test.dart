import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/mail_activity.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'support/preview_repository.dart';

Future<void> loadPreviewFonts() async {
  await (FontLoader(
    'Roboto',
  )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
  await (FontLoader(
    'NotoSans',
  )..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))).load();
  await (FontLoader(
    'NotoSans',
  )..addFont(rootBundle.load('assets/NotoSans-SemiBold.ttf'))).load();
}

class VisualSettings implements SettingsStore {
  @override
  Future<Preferences> read() async => const Preferences();

  @override
  Future<void> write(Preferences preferences) async {}
}

class ActivityPreviewRepository extends PreviewRepository
    implements MailActivityRepository {
  ActivityPreviewRepository({this.failRunnable = false})
    : super(delay: Duration.zero);

  bool failRunnable;

  final actions = <MailActivity>[
    MailActivity({
      'id': 'waiting',
      'mail': '1',
      'status': 'waiting',
      'fields': {'folder': 'Archive'},
      'error': 'Reconnect Personal to continue.',
    }),
    MailActivity({
      'id': 'uncertain',
      'mail': '2',
      'status': 'uncertain',
      'fields': {'starred': true},
      'error': 'Check the provider before retrying this flag.',
    }),
    ...List.generate(
      49,
      (index) => MailActivity({
        'id': 'done-$index',
        'mail': '${index + 10}',
        'status': 'succeeded',
        'fields': {'unread': false},
      }),
    ),
  ];

  @override
  Future<List<MailActivity>> mailActions({int offset = 0}) async =>
      actions.skip(offset).take(50).toList();

  @override
  Future<List<MailActivity>> runnableMailActions({
    int? afterCreated,
    String? afterId,
  }) async {
    if (failRunnable) throw StateError('The local activity read failed.');
    return const [];
  }

  @override
  Future<void> resumeMailAction(MailActivity action) async {}

  @override
  Future<void> cancelMailAction(String id) async {
    actions.removeWhere((action) => action.id == id);
  }

  @override
  Future<void> undoMailAction(
    MailActivity action, {
    void Function()? onAdmitted,
  }) async {
    onAdmitted?.call();
  }

  @override
  Future<void> inspectMailAction(MailActivity action) async {}
}

Future<void> renderActivity(
  WidgetTester tester,
  ThemeMode mode, {
  ActivityPreviewRepository? repository,
}) async {
  await loadPreviewFonts();
  tester.view.physicalSize = const Size(390, 700);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  final workspace = Workspace(
    repository ?? ActivityPreviewRepository(),
    VisualSettings(),
  );
  addTearDown(workspace.dispose);
  await workspace.initialize();
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: shepTheme(Brightness.light),
      darkTheme: shepTheme(Brightness.dark),
      themeMode: mode,
      home: MailActivityScreen(workspace: workspace),
    ),
  );
  await tester.pump();
}

void main() {
  testWidgets('compact Activity query failure has visible Retry', (
    tester,
  ) async {
    final repository = ActivityPreviewRepository(failRunnable: true);
    await renderActivity(tester, ThemeMode.dark, repository: repository);
    expect(find.widgetWithText(TextButton, 'Retry'), findsOneWidget);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/mail_activity_query_retry_dark.png'),
    );
    repository.failRunnable = false;
    await tester.tap(find.widgetWithText(TextButton, 'Retry'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextButton, 'Retry'), findsNothing);
    expect(find.text('Reconnect Personal to continue.'), findsOneWidget);
  });

  testWidgets('compact Mail Activity visual evidence', (tester) async {
    await renderActivity(tester, ThemeMode.light);
    expect(find.text('Cancel'), findsOneWidget);
    expect(find.text('Review'), findsOneWidget);
    expect(find.widgetWithText(TextButton, 'Undo'), findsWidgets);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/mail_activity_compact_light.png'),
    );
  });

  testWidgets('dark paged Mail Activity visual evidence', (tester) async {
    await renderActivity(tester, ThemeMode.dark);
    await tester.scrollUntilVisible(
      find.text('Load more'),
      500,
      scrollable: find.byType(Scrollable),
    );
    expect(find.text('Load more'), findsOneWidget);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/mail_activity_compact_dark_page.png'),
    );
  });
}
