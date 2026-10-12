import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/data/preferences_search_native.dart';
import 'package:shep_mobile/model/preferences_search.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'workspace_test.dart' show MemorySettings;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final name = Platform.isWindows
        ? 'shep_mobile_native.dll'
        : Platform.isMacOS
        ? 'libshep_mobile_native.dylib'
        : 'libshep_mobile_native.so';
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/${Platform.operatingSystem}/$name',
      ),
    );
  });
  test(
    'actual FFI uses shared accents, abbreviations, typos and numeric tokens',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-preferences-search-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite3',
      );
      final matcher = NativePreferenceSearchMatcher(repository);
      const entries = [
        PreferenceSearchEntry(
          target: 'theme',
          label: 'Theme',
          section: 'Appearance',
        ),
        PreferenceSearchEntry(
          target: 'profiles',
          label: 'Saved profiles',
          section: 'Connections',
        ),
        PreferenceSearchEntry(
          target: 'numbers',
          label: 'Office365',
          section: 'Connections',
        ),
        PreferenceSearchEntry(
          target: 'unicode',
          label: 'ガ 한글 Café',
          section: 'Reading',
        ),
      ];
      for (final (query, target) in [
        ('APPEARÁNCE', 'theme'),
        ('prf', 'profiles'),
        ('theem', 'theme'),
        ('office365', 'numbers'),
        ('カ\u3099', 'unicode'),
        ('한글', 'unicode'),
        ('cafe', 'unicode'),
      ]) {
        expect(
          (await matcher.match(entries, query)).single.target,
          target,
          reason: query,
        );
      }
      for (final query in [
        'office36',
        'office365x',
        'theme office365',
        'a' * 257,
      ]) {
        expect(await matcher.match(entries, query), isEmpty, reason: query);
      }
    },
  );
  testWidgets(
    'production native Preferences searches and reveals the real control through FFI',
    (tester) async {
      final opened = await tester.runAsync(() async {
        final directory = await Directory.systemTemp.createTemp(
          'shep-preferences-controls-',
        );
        final repository = await NativeRepository.open(
          '${directory.path}/mail.sqlite3',
        );
        return (directory, repository);
      });
      if (opened == null) fail('Native profile did not open.');
      final (directory, repository) = opened;
      addTearDown(() => directory.delete(recursive: true));
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: PreferencesView(workspace: workspace)),
        ),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const ValueKey('preferences-search')),
        'APPEARÁNCE',
      );
      final result = find.byKey(
        const ValueKey('preferences-result-preference-theme-Theme'),
      );
      for (
        var attempt = 0;
        attempt < 50 && result.evaluate().isEmpty;
        attempt++
      ) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
      expect(result, findsOneWidget);
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
    },
  );
}
