import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/data/preferences_search_native.dart';
import 'package:shep_mobile/model/preferences_search.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'package:shep_mobile/ui/theme.dart';
import '../test/workspace_test.dart' show MemorySettings;

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('native Preferences shared search and control reveal', (
    tester,
  ) async {
    final temporary = await getTemporaryDirectory();
    final directory = await Directory(
      temporary.path,
    ).createTemp('shep-preferences-search-');
    final repository = await NativeRepository.open(
      '${directory.path}/mail.sqlite3',
    );
    final workspace = Workspace(repository, MemorySettings());
    var cleaned = false;
    Future<void> cleanup() async {
      if (cleaned) return;
      cleaned = true;
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
      repository.profile.dispose();
      await directory.delete(recursive: true);
    }

    addTearDown(cleanup);
    final matcher = NativePreferenceSearchMatcher(repository);
    const entries = [
      PreferenceSearchEntry(
        target: 'theme',
        label: 'Theme',
        section: 'Appearance',
      ),
      PreferenceSearchEntry(
        target: 'profile',
        label: 'Saved profiles',
        section: 'Connections',
      ),
      PreferenceSearchEntry(
        target: 'numbers',
        label: 'Office365',
        section: 'Connections',
      ),
    ];
    for (final (query, target) in [
      ('APPEARÁNCE', 'theme'),
      ('prf', 'profile'),
      ('theem', 'theme'),
      ('office365', 'numbers'),
    ]) {
      expect((await matcher.match(entries, query)).single.target, target);
    }
    expect(await matcher.match(entries, 'office36'), isEmpty);
    expect(await matcher.match(entries, 'office365x'), isEmpty);
    await tester.pumpWidget(
      ListenableBuilder(
        listenable: workspace,
        builder: (_, _) => MaterialApp(
          theme: shepTheme(Brightness.light),
          darkTheme: shepTheme(Brightness.dark),
          themeMode: workspace.preferences.appearance,
          home: Scaffold(body: PreferencesView(workspace: workspace)),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final search = find.byKey(const ValueKey('preferences-search'));
    final result = find.byKey(
      const ValueKey('preferences-result-preference-theme-Theme'),
    );
    await tester.enterText(search, 'APPEARÁNCE');
    final deadline = DateTime.now().add(const Duration(seconds: 20));
    while (result.evaluate().isEmpty) {
      if (DateTime.now().isAfter(deadline)) {
        fail('Native Preferences search did not complete.');
      }
      await tester.pump(const Duration(milliseconds: 100));
    }
    if (Platform.isAndroid) await binding.convertFlutterSurfaceToImage();
    await tester.pumpAndSettle();
    await binding.takeScreenshot('native-preferences-search-accent');
    await tester.tap(result);
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('preference-theme')).hitTestable(),
      findsOneWidget,
    );
    await tester.tap(find.byType(DropdownButton<ThemeMode>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Dark').last);
    await tester.pumpAndSettle();
    expect(workspace.preferences.appearance, ThemeMode.dark);
    await binding.takeScreenshot('native-preferences-search-revealed-dark');
    binding.reportData = {
      ...?binding.reportData,
      'scenarios': ['native-preferences-shared-search-controls'],
    };
    await cleanup();
  });
}
