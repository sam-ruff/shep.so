import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'profile_settings_test.dart' show Bytes;
import 'support/preview_repository.dart';

void main() {
  test(
    'local edit while import is pending preserves both intents in UI and storage',
    () async {
      final bytes = Bytes();
      final store = DeviceSettings(storage: bytes);
      final workspace = Workspace(PreviewRepository(), store);
      addTearDown(workspace.dispose);
      final application = WorkspaceProfileApplication(workspace);
      final baseline = await application.captureProfilePreferences();
      bytes.hold = Completer<void>();
      final pending = application.applyProfilePreferences({
        'id': 'held',
        'baseline': baseline.toJson(),
        'changes': {'appearance': 'Dark', 'tooltips': false},
      });
      expect(workspace.preferences.appearance, ThemeMode.dark);
      await bytes.started.future;
      final local = workspace.savePreferences(
        workspace.preferences.copy(previewLines: 4),
      );
      bytes.hold!.complete();
      final receipt = await pending;
      await local;
      expect(receipt.applied, ['appearance', 'tooltips']);
      expect(workspace.preferences.appearance, ThemeMode.dark);
      expect(workspace.preferences.previewLines, 4);
      expect(workspace.preferences.tooltips, false);
      expect((await store.read()).encode(), workspace.preferences.encode());
      await workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.light),
      );
      await application.applyProfilePreferences({
        'id': 'held',
        'baseline': baseline.toJson(),
        'changes': {'appearance': 'Dark', 'tooltips': false},
      });
      expect(workspace.preferences.appearance, ThemeMode.light);
      expect((await store.read()).appearance, ThemeMode.light);
    },
  );
  test(
    'failure rolls back only untouched fields and retry saves dirty local values',
    () async {
      final bytes = Bytes();
      final store = DeviceSettings(storage: bytes);
      final workspace = Workspace(PreviewRepository(), store);
      addTearDown(workspace.dispose);
      final application = WorkspaceProfileApplication(workspace);
      final baseline = await application.captureProfilePreferences();
      bytes.hold = Completer<void>();
      bytes.failBefore = true;
      final pending = application.applyProfilePreferences({
        'id': 'failed',
        'baseline': baseline.toJson(),
        'changes': {'appearance': 'Dark', 'tooltips': false},
      });
      final failure = expectLater(pending, throwsStateError);
      await bytes.started.future;
      final local = workspace.savePreferences(
        workspace.preferences.copy(
          appearance: ThemeMode.light,
          previewLines: 4,
        ),
      );
      bytes.hold!.complete();
      await failure;
      await local;
      expect(workspace.preferences.appearance, ThemeMode.light);
      expect(workspace.preferences.tooltips, true);
      expect(workspace.preferences.previewLines, 4);
      expect((await store.read()).encode(), workspace.preferences.encode());
      bytes.failBefore = true;
      await workspace.savePreferences(
        workspace.preferences.copy(tooltips: false),
      );
      expect(workspace.error, contains('Could not save'));
      await expectLater(
        application.captureProfilePreferences(),
        throwsA(isA<Object>()),
      );
      await workspace.savePreferences(workspace.preferences);
      expect(workspace.error, isNull);
      expect((await store.read()).tooltips, false);
    },
  );
  test(
    'an ABA preference changed after review is kept while another field imports',
    () async {
      final store = DeviceSettings(storage: Bytes());
      final workspace = Workspace(PreviewRepository(), store);
      addTearDown(workspace.dispose);
      final application = WorkspaceProfileApplication(workspace);
      final baseline = await application.captureProfilePreferences();
      await workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.dark),
      );
      await workspace.savePreferences(const Preferences());
      final receipt = await application.applyProfilePreferences({
        'id': 'aba',
        'baseline': baseline.toJson(),
        'changes': {'appearance': 'Light', 'sender_pictures': false},
      });
      expect(receipt.kept, ['appearance']);
      expect(workspace.preferences.appearance, ThemeMode.system);
      expect(workspace.preferences.avatars, false);
    },
  );
}
