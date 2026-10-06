import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/group_history.dart';
import 'package:shep_mobile/model/mail_groups.dart';
import 'support/history_repository.dart';
import 'mail_selection_test.dart' show settled;

void main() {
  late HistoryRepository repository;
  late MailGroups groups;
  late GroupHistory history;
  setUp(() {
    repository = HistoryRepository();
    groups = MailGroups(
      repository: repository,
      changed: () {},
      refreshMail: () async {},
    );
    history = GroupHistory(groups: groups, changed: () {});
  });
  tearDown(() {
    history.dispose();
    groups.dispose();
  });

  test(
    'group and item pages remain bounded through every next/previous page',
    () async {
      await history.load();
      final original = history.jobs.first.id;
      history.toggle(history.jobs.first);
      await settled(() => !history.itemsLoading);
      final job = history.selectedJob!;
      expect(history.rows.length, 50);
      await history.loadItems(job, cursor: history.nextAfter);
      expect(history.rows.first.position, 50);
      expect(history.rows.length, 50);
      await history.loadItems(job, cursor: history.nextAfter);
      expect(history.rows.length, 25);
      expect(history.nextAfter, isNull);
      await history.loadItems(job, cursor: history.previousAfter);
      expect(history.rows.first.position, 50);
      await history.loadItems(job, cursor: history.previousAfter);
      expect(history.rows.first.position, 0);
      expect(history.hasPrevious, false);
      for (var i = 0; i < 3; i++) {
        await history.load(cursor: history.nextBefore);
        expect(history.jobs.length, lessThanOrEqualTo(20));
        expect(history.rows, isEmpty);
      }
      expect(history.jobs.length, 5);
      expect(history.nextBefore, isNull);
      await history.load(cursor: history.previousBefore);
      expect(history.jobs.first.id, 'group-024');
      await history.load(cursor: history.previousBefore);
      await history.load(cursor: history.previousBefore);
      expect(history.jobs.first.id, original);
      expect(repository.records.length, 65);
    },
  );

  for (final accept in [false, true]) {
    for (final sameJob in [false, true]) {
      test(
        'late ${accept ? 'Accept' : 'Retry'} cannot reload ${sameJob ? 'an away-and-back page' : 'another group'}',
        () async {
          await history.load();
          final old = history.jobs.first;
          history.toggle(old);
          await settled(() => !history.itemsLoading);
          final item = history.rows[accept ? 1 : 0];
          repository.heldDecisionKind = accept ? 'accept' : 'retry';
          repository.heldDecisionJob = old.id;
          repository.decisionHold = Completer<void>();
          final deciding = history.decide(
            old,
            item,
            history.detailGeneration,
            accept: accept,
          );
          await repository.decisionStarted.future;
          history.toggle(history.jobs[1]);
          await settled(() => !history.itemsLoading);
          if (sameJob) {
            history.toggle(old);
            await settled(() => !history.itemsLoading);
            await history.loadItems(old, cursor: history.nextAfter);
          }
          final selected = history.selected, after = history.after;
          final rows = history.rows;
          repository.decisionHold!.complete();
          await deciding;
          expect(history.selected, selected);
          expect(history.after, after);
          expect(history.rows, same(rows));
          final next = history.selectedJob!;
          final target = history.rows.firstWhere((i) => i.canRetry);
          await history.decide(
            next,
            target,
            history.detailGeneration,
            accept: false,
          );
          expect(repository.decisions.last, (
            'retry',
            next.id,
            target.position,
            target.mail,
          ));
        },
      );
    }
  }

  test(
    'failed page retry uses its retained cursor and an exact final50 has no next',
    () async {
      repository = HistoryRepository(items: 100);
      groups.dispose();
      history.dispose();
      groups = MailGroups(
        repository: repository,
        changed: () {},
        refreshMail: () async {},
      );
      history = GroupHistory(groups: groups, changed: () {});
      await history.load();
      final job = history.jobs.first;
      history.toggle(job);
      await settled(() => !history.itemsLoading);
      repository.failItems = true;
      await history.loadItems(job, cursor: history.nextAfter);
      expect(history.rows, isEmpty);
      expect(history.after, 49);
      expect(history.itemsError, contains('Retry this page'));
      await history.loadItems(job, cursor: history.after);
      expect(history.rows.length, 50);
      expect(history.nextAfter, isNull);
    },
  );

  test(
    'closing and disposal fence late reads and decision follow-ups',
    () async {
      await history.load();
      final job = history.jobs.first;
      repository.heldItemsJob = job.id;
      repository.itemsHold = Completer<void>();
      history.toggle(job);
      history.toggle(job);
      repository.itemsHold!.complete();
      await settled(
        () => repository.calls.where((c) => c['kind'] == 'items').isNotEmpty,
      );
      await Future<void>.delayed(Duration.zero);
      expect(history.rows, isEmpty);
      expect(history.selected, isNull);
      history.toggle(job);
      await settled(() => !history.itemsLoading);
      final row = history.rows.first;
      repository.heldDecisionKind = 'retry';
      repository.heldDecisionJob = job.id;
      repository.decisionHold = Completer<void>();
      final deciding = history.decide(
        job,
        row,
        history.detailGeneration,
        accept: false,
      );
      await repository.decisionStarted.future;
      final reads = repository.calls.where((c) => c['kind'] == 'items').length;
      history.dispose();
      repository.decisionHold!.complete();
      await deciding;
      expect(history.rows, isEmpty);
      expect(history.jobs, isEmpty);
      expect(repository.calls.where((c) => c['kind'] == 'items').length, reads);
    },
  );

  test(
    'older active and attention records remain owned outside the visible page',
    () async {
      repository.records['group-000']!['state'] = 'paused';
      await groups.refreshHistory();
      expect(groups.jobs.length, 20);
      expect(groups.jobs.any((j) => j.id == 'group-000'), false);
      expect(groups.activeJobs.single.id, 'group-000');
      expect(groups.attentionCount, 65 * 125);
      await history.load(cursor: 25);
      expect(history.jobs.length, 20);
      expect(groups.activeJobs.single.id, 'group-000');
      repository.records['group-000']!['state'] = 'finished';
      repository.records['group-000']!['revision'] = 2;
      await groups.refreshHistory();
      expect(groups.completed!.id, 'group-000');
    },
  );

  test('rapid Refresh shares one read and its latest replacement', () async {
    await history.load();
    repository.historyHold = Completer<void>();
    final held = history.load(preserve: true);
    for (var n = 0; n < 100; n++) {
      expect(identical(history.load(cursor: 25, preserve: true), held), true);
    }
    expect(repository.calls.where((c) => c['kind'] == 'history').length, 2);
    repository.historyHold!.complete();
    await held;
    expect(repository.calls.where((c) => c['kind'] == 'history').length, 3);
    expect(history.jobs.first.id, 'group-023');
    expect(history.jobs.length, 20);
  });

  test(
    'two removals invalidate a held page without resurrecting either job',
    () async {
      await history.load();
      repository.historyHold = Completer<void>();
      final held = history.load(preserve: true);
      final removed = history.jobs.take(2).toList();
      for (final job in removed) {
        await history.remove(job);
      }
      expect(history.jobs.any((j) => removed.any((r) => r.id == j.id)), false);
      repository.historyHold!.complete();
      await held;
      expect(history.jobs.any((j) => removed.any((r) => r.id == j.id)), false);
      expect(history.jobs.length, 20);
      expect(repository.records.length, 63);
    },
  );

  test(
    'failed older-page read retains usable known navigation and explicit retry',
    () async {
      await history.load();
      final next = history.nextBefore;
      repository.failHistory = true;
      await history.load(cursor: next);
      expect(history.jobs.first.id, 'group-064');
      expect(history.nextBefore, next);
      expect(history.retryBefore, next);
      expect(history.error, contains('Retry the requested page'));
      await history.load(cursor: history.retryBefore, preserve: true);
      expect(history.jobs.first.id, 'group-044');
      expect(history.error, isNull);
    },
  );

  test(
    'older attention opens its exact target directly and an empty page can go back',
    () async {
      await groups.refreshHistory();
      await history.openTarget(groups.attentionTarget!);
      await settled(() => !history.itemsLoading);
      expect(history.selected, 'group-000');
      expect(history.rows.first.mail, 'group-000-mail-0');
      repository.records.removeWhere((id, _) => id != 'group-000');
      await history.load(cursor: 1);
      expect(history.jobs, isEmpty);
      expect(history.hasNewer, true);
      await history.load(cursor: history.previousBefore);
      expect(history.jobs.single.id, 'group-000');
    },
  );
  test(
    'held owner observation cannot resurrect two removals or overwrite newer Pause',
    () async {
      repository.records['group-064']!['state'] = 'running';
      await groups.refreshHistory();
      final active = groups.activeJobs.single;
      final removed = groups.jobs
          .where((j) => j.id != active.id)
          .take(2)
          .toList();
      repository.ownerHold = Completer<void>();
      repository.reportRunnable = true;
      final reading = groups.refreshHistory();
      for (final job in removed) {
        expect(await groups.remove(job), true);
      }
      await groups.pause(active);
      repository.reportRunnable = false;
      repository.ownerHold!.complete();
      await reading;
      expect(groups.jobs.any((j) => removed.any((r) => r.id == j.id)), false);
      expect(groups.activeJobs.single.state, 'paused');
      expect(groups.jobs.length, 20);
      expect(repository.stepCalls, 0);
    },
  );

  test('owner Refresh coalesces onto one shared idle future', () async {
    repository.ownerHold = Completer<void>();
    final held = groups.refreshHistory();
    for (var n = 0; n < 100; n++) {
      expect(identical(groups.refreshHistory(), held), true);
    }
    expect(repository.calls.where((c) => c['kind'] == 'history').length, 1);
    repository.ownerHold!.complete();
    await held;
    expect(repository.calls.where((c) => c['kind'] == 'history').length, 2);
    expect(groups.historyLoading, false);
  });
}
