import 'dart:async';
import 'dart:convert';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/data/profile_settings.dart';
import 'package:shep_mobile/model/preferences.dart';

class Bytes implements PreferenceStorage {
  String? value;
  bool failBefore = false, failAfter = false;
  Completer<void>? hold;
  final started = Completer<void>();
  @override
  Future<String?> read() async => value;
  @override
  Future<void> write(String next) async {
    if (!started.isCompleted) started.complete();
    if (hold case final gate?) await gate.future;
    if (failBefore) {
      failBefore = false;
      throw StateError('storage unavailable');
    }
    value = next;
    if (failAfter) {
      failAfter = false;
      throw StateError('reply lost');
    }
  }
}

void main() {
  test(
    'legacy preferences, field ABA and unrelated edits survive profile application',
    () async {
      final bytes = Bytes()
        ..value = const Preferences(appearance: ThemeMode.light).encode();
      final store = DeviceSettings(storage: bytes);
      final baseline = await store.profileSnapshot();
      await store.write(baseline.preferences.copy(appearance: ThemeMode.dark));
      await store.write(
        baseline.preferences.copy(appearance: ThemeMode.light, previewLines: 4),
      );
      final applied = await store.applyProfile(
        id: 'enrollment-one',
        baseline: baseline,
        changes: {'appearance': 'Dark', 'tooltips': false},
      );
      expect(applied.applied, ['tooltips']);
      expect(applied.kept, ['appearance']);
      expect(applied.preferences.appearance, ThemeMode.light);
      expect(applied.preferences.previewLines, 4);
      expect(applied.preferences.tooltips, isFalse);
      final reopened = DeviceSettings(storage: bytes);
      expect((await reopened.read()).encode(), applied.preferences.encode());
      await reopened.write(applied.preferences.copy(tooltips: true));
      final retried = await reopened.applyProfile(
        id: 'enrollment-one',
        baseline: baseline,
        changes: {'tooltips': false, 'appearance': 'Dark'},
      );
      expect(retried.applied, ['tooltips']);
      expect(retried.preferences.tooltips, isTrue);
      await expectLater(
        reopened.applyProfile(
          id: 'enrollment-one',
          baseline: baseline,
          changes: {'appearance': 'Dark'},
        ),
        throwsFormatException,
      );
    },
  );
  test(
    'lost platform receipt is verified and definite failure retries the same plan',
    () async {
      final bytes = Bytes();
      final store = DeviceSettings(storage: bytes);
      final baseline = await store.profileSnapshot();
      bytes.failBefore = true;
      await expectLater(
        store.applyProfile(
          id: 'first',
          baseline: baseline,
          changes: {'appearance': 'Dark'},
        ),
        throwsStateError,
      );
      expect(bytes.value, isNull);
      bytes.failAfter = true;
      final result = await store.applyProfile(
        id: 'first',
        baseline: baseline,
        changes: {'appearance': 'Dark'},
      );
      expect(result.applied, ['appearance']);
      expect((await store.read()).appearance, ThemeMode.dark);
      final saved = bytes.value;
      await DeviceSettings(storage: bytes).applyProfile(
        id: 'first',
        baseline: baseline,
        changes: {'appearance': 'Dark'},
      );
      expect(bytes.value, saved);
    },
  );
  test(
    'shared handles serialize a pending save before applying frozen values',
    () async {
      final bytes = Bytes();
      final one = DeviceSettings(storage: bytes),
          two = DeviceSettings(storage: bytes);
      final baseline = await one.profileSnapshot();
      bytes.hold = Completer<void>();
      final write = one.write(const Preferences(appearance: ThemeMode.dark));
      await bytes.started.future;
      var finished = false;
      final apply = two
          .applyProfile(
            id: 'held',
            baseline: baseline,
            changes: {'appearance': 'Light', 'sender_pictures': false},
          )
          .then((v) {
            finished = true;
            return v;
          });
      await Future<void>.delayed(Duration.zero);
      expect(finished, isFalse);
      bytes.hold!.complete();
      await write;
      final result = await apply;
      expect(result.kept, ['appearance']);
      expect(result.applied, ['sender_pictures']);
      expect(result.preferences.appearance, ThemeMode.dark);
      expect(result.preferences.avatars, isFalse);
    },
  );
  test('queued reviews freeze both changes and baseline revisions', () async {
    final bytes = Bytes();
    final store = DeviceSettings(storage: bytes);
    final captured = await store.profileSnapshot();
    final revisions = Map<String, int>.of(captured.revisions);
    final baseline = ProfileSettingsSnapshot(captured.preferences, revisions);
    bytes.hold = Completer<void>();
    final pending = store.write(const Preferences(appearance: ThemeMode.dark));
    await bytes.started.future;
    final changes = <String, Object?>{'tooltips': false};
    final apply = store.applyProfile(
      id: 'frozen',
      baseline: baseline,
      changes: changes,
    );
    changes['tooltips'] = true;
    revisions['tooltips'] = 999;
    bytes.hold!.complete();
    await pending;
    final receipt = await apply;
    expect(receipt.applied, ['tooltips']);
    expect(receipt.preferences.tooltips, isFalse);
    expect(receipt.preferences.appearance, ThemeMode.dark);
  });
  test(
    'independent preference storage remains usable while another owner is blocked',
    () async {
      final held = Bytes()..hold = Completer<void>();
      final first = DeviceSettings(storage: held).write(const Preferences());
      await held.started.future;
      final separate = DeviceSettings(storage: Bytes());
      final snapshot = await separate.profileSnapshot();
      expect(snapshot.preferences.previewLines, 2);
      await separate.saveLocal({'tooltips': false});
      expect((await separate.read()).tooltips, false);
      held.hold!.complete();
      await first;
    },
  );
  test(
    'explicit reset uses defaults and unknown metadata is never overwritten',
    () async {
      final bytes = Bytes()
        ..value = jsonEncode({
          ...jsonDecode(
                const Preferences(previewLines: 4, tooltips: false).encode(),
              )
              as Map,
          'future_optional': {'retained': true},
        });
      final store = DeviceSettings(storage: bytes);
      final baseline = await store.profileSnapshot();
      final result = await store.applyProfile(
        id: 'reset',
        baseline: baseline,
        changes: {'preview_lines': null, 'tooltips': true},
      );
      expect(result.preferences.previewLines, 2);
      expect(result.preferences.tooltips, isTrue);
      expect(jsonDecode(bytes.value!)['future_optional'], {'retained': true});
      final before = bytes.value;
      await expectLater(
        store.applyProfile(
          id: 'unknown',
          baseline: baseline,
          changes: {'credential_slot': 'untrusted'},
        ),
        throwsFormatException,
      );
      expect(bytes.value, before);
      final bad = jsonDecode(bytes.value!);
      bad['_profile_preferences']['version'] = 99;
      bytes.value = jsonEncode(bad);
      final unknown = bytes.value;
      await expectLater(
        store.write(const Preferences()),
        throwsFormatException,
      );
      expect(bytes.value, unknown);
    },
  );
}
