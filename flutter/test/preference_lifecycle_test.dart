import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'support/preview_repository.dart';

class HeldSettings implements SettingsStore {
  Preferences value = const Preferences();
  final writes = <Completer<void>>[];

  @override
  Future<Preferences> read() async => value;

  @override
  Future<void> write(Preferences next) async {
    final held = Completer<void>();
    writes.add(held);
    await held.future;
    value = next;
  }
}

void main() {
  for (final brightness in Brightness.values) {
    testWidgets('compact preference failure in ${brightness.name}', (
      tester,
    ) async {
      await tester.binding.setSurfaceSize(const Size(390, 700));
      addTearDown(() => tester.binding.setSurfaceSize(null));
      await (FontLoader(
        'Roboto',
      )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
      await (FontLoader('NotoSans')
            ..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))
            ..addFont(rootBundle.load('assets/NotoSans-SemiBold.ttf')))
          .load();
      final settings = HeldSettings();
      final workspace = Workspace(PreviewRepository(), settings);
      addTearDown(workspace.dispose);
      final pending = workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.dark),
      );
      await tester.pump();
      settings.writes.single.completeError(StateError('disk full'));
      await pending;
      await tester.pumpWidget(
        MaterialApp(
          theme: shepTheme(brightness),
          home: Scaffold(body: PreferencesView(workspace: workspace)),
        ),
      );
      await tester.pump();
      expect(find.text('Retry save'), findsOneWidget);
      await expectLater(
        find.byType(Scaffold),
        matchesGoldenFile(
          'goldens/preference_save_failure_${brightness.name}.png',
        ),
      );
    });
  }
  test('older preference failure preserves a newer local choice', () async {
    final settings = HeldSettings();
    final workspace = Workspace(PreviewRepository(), settings);
    addTearDown(workspace.dispose);
    final older = workspace.savePreferences(
      workspace.preferences.copy(appearance: ThemeMode.dark),
    );
    await Future<void>.delayed(Duration.zero);
    final newer = workspace.savePreferences(
      workspace.preferences.copy(previewLines: 0),
    );
    settings.writes.first.completeError(StateError('disk full'));
    await older;
    await Future<void>.delayed(Duration.zero);
    expect(workspace.preferenceSaveError, isNull);
    expect(workspace.savingPreferences, isTrue);
    settings.writes.last.complete();
    await newer;
    expect(settings.value.appearance, ThemeMode.dark);
    expect(settings.value.previewLines, 0);
    expect(workspace.savingPreferences, isFalse);
  });

  testWidgets(
    'Preferences retains owned failure and Retry through navigation',
    (tester) async {
      final settings = HeldSettings();
      final workspace = Workspace(PreviewRepository(), settings);
      addTearDown(workspace.dispose);
      Widget screen(bool preferences) => MaterialApp(
        theme: shepTheme(Brightness.light),
        home: Scaffold(
          body: ListenableBuilder(
            listenable: workspace,
            builder: (_, _) => preferences
                ? PreferencesView(workspace: workspace)
                : const Text('Mail'),
          ),
        ),
      );
      await tester.pumpWidget(screen(true));
      final saved = workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.dark),
      );
      await tester.pump();
      expect(find.text('Saving preferences…'), findsOneWidget);
      expect(workspace.preferences.appearance, ThemeMode.dark);
      settings.writes.first.completeError(StateError('disk full'));
      await saved;
      await tester.pump();
      expect(find.text('Retry save'), findsOneWidget);
      workspace.error = 'An unrelated mail failure';
      void mailRetry() {}
      workspace.retry = mailRetry;
      await tester.pumpWidget(screen(false));
      await tester.pumpWidget(screen(true));
      expect(find.text('Retry save'), findsOneWidget);
      await tester.tap(find.text('Retry save'));
      await tester.pump();
      expect(find.text('Saving preferences…'), findsOneWidget);
      expect(
        tester
            .widget<TextButton>(find.widgetWithText(TextButton, 'Retry save'))
            .onPressed,
        isNull,
      );
      settings.writes.last.complete();
      await tester.pump();
      await tester.pump();
      expect(workspace.preferenceSaveError, isNull);
      expect(workspace.error, 'An unrelated mail failure');
      expect(workspace.retry, same(mailRetry));
      expect(settings.value.appearance, ThemeMode.dark);
      expect(find.text('Retry save'), findsNothing);
    },
  );
}
