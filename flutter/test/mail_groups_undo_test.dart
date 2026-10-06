import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/groups.dart';
import 'package:shep_mobile/model/mail_groups.dart';
import 'support/history_repository.dart';
import 'mail_selection_test.dart' show settled;

class _UndoRepository extends HistoryRepository {
  _UndoRepository({super.groups = 40}) : super(items: 1) {
    for (final rows in members.values) {
      rows.single['state'] = 'done';
    }
  }
  final lostUndos = <String>{}, failedInspections = <String>{};
  Completer<void>? undoHold, inspectHold;
  String? inspectTarget;
  final inspected = Completer<void>();

  @override
  Future<dynamic> groups(Map<String, Object?> command) async {
    final id = command['id'] as String?;
    if (command['kind'] == 'inspect') {
      calls.add(Map.of(command));
      if (failedInspections.remove(id)) {
        throw StateError('Inspection unavailable');
      }
      final result = summary(id!);
      if (inspectTarget == id && inspectHold != null) {
        if (!inspected.isCompleted) inspected.complete();
        await inspectHold!.future;
      }
      return result;
    }
    final result = await super.groups(command);
    if (command['kind'] == 'undo') {
      await undoHold?.future;
      if (lostUndos.remove(id)) throw StateError('Undo reply unavailable');
    }
    return result;
  }
}

void main() {
  MailGroups owner(_UndoRepository repository) => MailGroups(
    repository: repository,
    changed: () {},
    refreshMail: () async {},
  );

  test(
    '32 held decisions retain a visible exact latest capacity Retry',
    () async {
      final repository = _UndoRepository()..undoHold = Completer<void>();
      final groups = owner(repository);
      addTearDown(groups.dispose);
      final jobs = repository.records.keys
          .map((id) => GroupJob(repository.summary(id)))
          .toList();
      final held = [for (final job in jobs.take(32)) groups.undo(job)];
      await groups.undo(jobs[32]);
      expect(repository.calls.where((c) => c['kind'] == 'undo').length, 32);
      expect(groups.failedUndo!.id, jobs[32].id);
      expect(groups.undoError, contains('Undo is catching up'));
      expect(groups.undoPending(jobs[32]), true);
      repository.undoHold!.complete();
      await Future.wait(held);
      await settled(() => !groups.running && !groups.historyLoading);
      await groups.undo(groups.failedUndo!);
      await settled(() => !groups.running && !groups.historyLoading);
      expect(repository.calls.where((c) => c['kind'] == 'undo').length, 33);
      expect(groups.failedUndo, isNull);
      expect(groups.undoError, isNull);
    },
  );

  for (final fail in [false, true]) {
    test(
      'Remove fences a held Undo response ${fail ? 'error' : 'success'}',
      () async {
        final repository = _UndoRepository(groups: 1)
          ..undoHold = Completer<void>();
        final groups = owner(repository);
        addTearDown(groups.dispose);
        await groups.refreshHistory();
        final job = groups.jobs.single;
        if (fail) repository.lostUndos.add(job.id);
        final deciding = groups.undo(job);
        await settled(() => repository.records[job.id]!['undo'] == true);
        expect(await groups.remove(GroupJob(repository.summary(job.id))), true);
        repository.undoHold!.complete();
        await deciding;
        await settled(() => !groups.historyLoading);
        expect(groups.jobs, isEmpty);
        expect(groups.undoPending(job), false);
        expect(groups.undoError, isNull);
        expect(repository.calls.where((c) => c['kind'] == 'inspect'), isEmpty);
      },
    );

    test(
      'Remove retires only its Undo and fences late inspection ${fail ? 'error' : 'success'}',
      () async {
        final repository = _UndoRepository(groups: 2);
        final groups = owner(repository);
        addTearDown(groups.dispose);
        await groups.refreshHistory();
        final removed = groups.jobs.first;
        final retained = groups.jobs.last;
        repository.lostUndos.addAll([removed.id, retained.id]);
        repository.failedInspections.addAll([removed.id, retained.id]);
        await groups.undo(removed);
        await groups.undo(retained);
        expect(groups.failedUndo!.id, removed.id);
        repository.inspectTarget = removed.id;
        repository.inspectHold = Completer<void>();
        final inspecting = groups.undo(removed);
        await repository.inspected.future;
        expect(groups.failedUndo!.id, removed.id);
        expect(groups.undoDeciding(removed), true);
        expect(
          await groups.remove(GroupJob(repository.summary(removed.id))),
          true,
        );
        if (fail) {
          repository.inspectHold!.completeError(
            StateError('Late removed inspection'),
          );
        } else {
          repository.inspectHold!.complete();
        }
        await inspecting;
        await settled(() => !groups.historyLoading);
        expect(groups.undoPending(removed), false);
        expect(groups.failedUndo!.id, retained.id);
        expect(groups.undoError, contains(retained.title));
        expect(groups.jobs.any((j) => j.id == removed.id), false);
        await groups.undo(retained);
        expect(groups.undoError, isNull);
      },
    );
  }
}
