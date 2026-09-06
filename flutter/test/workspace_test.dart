import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'support/preview_repository.dart';
import 'support/paged_repository.dart';

class MemorySettings implements SettingsStore {
  Preferences value = const Preferences();
  bool fail = false;
  @override
  Future<Preferences> read() async => value;
  @override
  Future<void> write(Preferences p) async {
    if (fail) throw StateError('disk full');
    value = p;
  }
}

class ControlledRepository extends PreviewRepository {
  ControlledRepository() : super(delay: Duration.zero);
  final jobs = <Completer<void>>[];
  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    final job = Completer<void>();
    jobs.add(job);
    await job.future;
    await super.mutate(id, fields);
  }
}

Future<void> tick() => Future<void>.delayed(Duration.zero);
void main() {
  test(
    'paged refresh retains the metadata needed to Undo a moved row',
    () async {
      final repo = PagedRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      final moved = w.action('1', MailAction.archive);
      await tick();
      repo.jobs[0].complete();
      await moved;
      await w.loadPage();
      expect(w.mail('1'), isNull);
      expect(w.undo, isNotNull);
      w.undo!();
      expect(w.mail('1')!.folder, 'Inbox');
      await tick();
      expect(repo.jobs.length, 2);
      repo.jobs[1].complete();
      await tick();
      await tick();
      await tick();
      expect(repo.cached.firstWhere((m) => m.id == '1').folder, 'Inbox');
      expect(w.visible.any((m) => m.id == '1'), true);
      expect(w.error, isNull);
    },
  );

  test('archive removes immediately while persistence is pending', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final job = w.action('1', MailAction.archive);
    expect(w.visible.any((m) => m.id == '1'), false);
    expect(w.pending, 1);
    w.navigate('Archive');
    expect(w.visible.any((m) => m.id == '1'), true);
    await tick();
    repo.jobs.single.complete();
    await job;
    expect(w.pending, 0);
    expect(repo.cached.first.folder, 'Archive');
  });
  test('failed move rolls back and retains browsed folder', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final job = w.action('1', MailAction.archive);
    w.navigate('Sent');
    await tick();
    repo.jobs.single.completeError(StateError('reject'));
    await job;
    expect(w.folder, 'Sent');
    expect(w.mail('1')!.folder, 'Inbox');
    expect(w.error, contains('restored'));
  });
  test('old flag failure preserves newer read and flag intent', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final first = w.action('1', MailAction.star);
    final second = w.action('1', MailAction.star);
    final third = w.action('1', MailAction.read);
    expect(w.mail('1')!.starred, false);
    expect(w.mail('1')!.unread, false);
    await tick();
    repo.jobs[0].completeError(StateError('reject'));
    await first;
    await tick();
    repo.jobs[1].complete();
    await second;
    await tick();
    repo.jobs[2].complete();
    await third;
    expect(w.mail('1')!.starred, false);
    expect(repo.cached.first.unread, false);
  });
  test('queued failures converge to confirmed state', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final jobs = [
      w.action('1', MailAction.star),
      w.action('1', MailAction.star),
      w.action('1', MailAction.star),
    ];
    for (var i = 0; i < 3; i++) {
      await tick();
      repo.jobs[i].completeError(StateError('reject'));
      await jobs[i];
    }
    expect(w.mail('1')!.starred, false);
    expect(w.error, isNotNull);
  });
  test('undo serializes behind pending archive', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final first = w.action('1', MailAction.archive);
    w.undo!();
    expect(w.mail('1')!.folder, 'Inbox');
    await tick();
    repo.jobs[0].complete();
    await first;
    await tick();
    repo.jobs[1].complete();
    await tick();
    await tick();
    expect(repo.cached.first.folder, 'Inbox');
  });
  test('refresh cannot restore old pending action', () async {
    final repo = ControlledRepository();
    final w = Workspace(repo, MemorySettings());
    final job = w.action('1', MailAction.archive);
    await w.refresh();
    expect(w.mail('1')!.folder, 'Archive');
    await tick();
    repo.jobs[0].complete();
    await job;
  });
  test('latest preferences persist together', () async {
    final settings = MemorySettings();
    final w = Workspace(PreviewRepository(), settings);
    final a = w.savePreferences(w.preferences.copy(appearance: ThemeMode.dark));
    final b = w.savePreferences(
      w.preferences.copy(
        leftSwipe: MailAction.star,
        rightSwipe: MailAction.none,
        previewLines: 0,
      ),
    );
    await Future.wait([a, b]);
    final reopened = Workspace(PreviewRepository(), settings);
    await reopened.initialize();
    expect(reopened.preferences.appearance, ThemeMode.dark);
    expect(reopened.preferences.leftSwipe, MailAction.star);
    expect(reopened.preferences.rightSwipe, MailAction.none);
    expect(reopened.preferences.previewLines, 0);
  });
  test('preference failure retries current values', () async {
    final settings = MemorySettings()..fail = true;
    final w = Workspace(PreviewRepository(), settings);
    await w.savePreferences(w.preferences.copy(appearance: ThemeMode.dark));
    expect(w.error, contains('Retry'));
    settings.fail = false;
    w.retry!();
    await tick();
    await tick();
    expect(settings.value.appearance, ThemeMode.dark);
  });
  test('preview sending refusal retains draft', () async {
    final w = Workspace(
      PreviewRepository(delay: Duration.zero),
      MemorySettings(),
    );
    const draft = Draft(
      id: 'd1',
      to: 'test@example.test',
      subject: 'Test',
      body: 'Unsent',
    );
    expect(await w.saveDraft(draft), true);
    expect(await w.send(draft), false);
    expect(w.drafts['d1']!.body, 'Unsent');
  });
  test('filter and sort recompute after flags', () async {
    final w = Workspace(
      PreviewRepository(delay: Duration.zero),
      MemorySettings(),
    );
    w.setFilter('Flagged');
    expect(w.visible.length, 1);
    await w.action('1', MailAction.star);
    expect(w.visible.length, 2);
    w.sort();
    expect(w.visible.last.id, '1');
  });
  test('all swipe settings and disabled values round trip', () {
    for (final a in MailAction.values) {
      final p = Preferences.decode(
        Preferences(leftSwipe: a, rightSwipe: a).encode(),
      );
      expect(p.leftSwipe, a);
      expect(p.rightSwipe, a);
    }
    expect(() => Preferences.decode('{"version":2}'), throwsFormatException);
  });
}
