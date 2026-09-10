import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/move_feedback.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'workspace_test.dart'
    show ControlledRepository, MemorySettings, tick, waitUntil;

class CommittedUndoRepository extends ControlledRepository {
  int calls = 0;
  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    final call = ++calls;
    await super.mutate(id, fields);
    if (call == 2) {
      throw const MailOperationFailure(
        'Undo was acknowledged; refresh its saved metadata.',
        committed: true,
      );
    }
  }
}

void main() {
  test(
    'acknowledged Undo with a metadata warning can only refresh, never repeat',
    () async {
      final repo = CommittedUndoRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      final move = w.action('1', MailAction.archive);
      await waitUntil(() => repo.jobs.length == 1);
      repo.jobs.single.complete();
      await move;
      w.undo!();
      await waitUntil(() => repo.jobs.length == 2);
      repo.jobs[1].complete();
      await waitUntil(() => w.pending == 0);
      expect(w.undoFailures.single.restoreCommitted, true);
      w.retryUndos();
      await tick();
      expect(repo.calls, 2);
      await w.refreshRestored();
      expect(w.undoFailures, isEmpty);
      expect(repo.calls, 2);
      expect(repo.cached.first.folder, 'Inbox');
    },
  );
  testWidgets(
    'destination grouping refreshes six-second expiry and dismissal survives old failure',
    (tester) async {
      final feedback = MoveFeedback(() {});
      final first = feedback.add('1', 'work', 'Inbox', 'Archive');
      expect(feedback.label, 'Archived 1 message');
      await tester.pump(const Duration(seconds: 3));
      feedback.add('2', 'personal', 'Inbox', 'archive');
      expect(feedback.label, 'Archived 2 messages');
      await tester.pump(const Duration(seconds: 3));
      expect(feedback.label, 'Archived 2 messages');
      feedback.failed(first);
      expect(feedback.label, 'Archived 1 message');
      await tester.pump(const Duration(seconds: 3));
      expect(feedback.label, isNull);
      feedback.add('3', 'work', 'Inbox', 'Plans');
      feedback.add('4', 'personal', 'Inbox', 'Plans');
      expect(feedback.label, 'Moved 1 message to Plans');
      feedback.add('5', 'personal', 'Inbox', 'Plans');
      expect(feedback.label, 'Moved 2 messages to Plans');
      feedback.add('6', 'personal', 'Plans', 'INBOX');
      expect(feedback.label, 'Moved 1 message to Inbox');
      feedback.dismiss();
      feedback.failed(first);
      expect(feedback.label, isNull);
      feedback.dispose();
    },
  );

  test(
    'grouped partial move failure and retryable Undo retain exact destinations',
    () async {
      final repo = ControlledRepository();
      final model = Workspace(repo, MemorySettings());
      addTearDown(model.dispose);
      final first = model.action('1', MailAction.archive);
      final second = model.action('2', MailAction.archive);
      await waitUntil(() => repo.jobs.length == 2);
      expect(model.moves.label, 'Archived 2 messages');
      repo.jobs[0].completeError(StateError('First rejected'));
      await first;
      expect(model.moves.label, 'Archived 1 message');
      model.undo!();
      expect(model.moves.label, 'Restored 1 message');
      expect(model.mail('2')!.folder, 'Inbox');
      expect(repo.jobs.length, 2);
      repo.jobs[1].complete();
      await second;
      await waitUntil(() => repo.jobs.length == 3);
      repo.jobs[2].completeError(StateError('Undo rejected'));
      await waitUntil(() => model.pending == 0);
      expect(model.mail('2')!.folder, 'Archive');
      expect(model.undoFailures.length, 1);
      expect(model.moves.label, isNull);
      model.retryUndos();
      expect(model.mail('2')!.folder, 'Inbox');
      await waitUntil(() => repo.jobs.length == 4);
      repo.jobs[3].complete();
      await waitUntil(() => model.pending == 0);
      expect(model.undoFailures, isEmpty);
      expect(model.error, isNull);
      expect(repo.cached.firstWhere((m) => m.id == '2').folder, 'Inbox');
      expect(model.moves.label, 'Restored 1 message');
    },
  );

  test(
    'Undo before dispatch cancels the unsent move and sends no reverse',
    () async {
      final repo = ControlledRepository();
      final model = Workspace(repo, MemorySettings());
      addTearDown(model.dispose);
      model.beginReading('1');
      final move = model.action('1', MailAction.archive);
      await waitUntil(() => repo.jobs.length == 1); // Read, before MOVE.
      model.undo!();
      expect(model.mail('1')!.folder, 'Inbox');
      expect(model.moves.label, 'Restored 1 message');
      repo.jobs.single.complete();
      await move;
      await waitUntil(() => model.pending == 0);
      expect(repo.jobs.length, 1);
      expect(repo.cached.first.folder, 'Inbox');
      expect(repo.cached.first.unread, false);
    },
  );

  test(
    'old Undo and late failure cannot affect a newer group; dismissal sticks',
    () async {
      final repo = ControlledRepository();
      final model = Workspace(repo, MemorySettings());
      addTearDown(model.dispose);
      final first = model.action('1', MailAction.archive);
      await tick();
      final oldUndo = model.undo;
      final second = model.action('2', MailAction.trash);
      await tick();
      final newUndo = model.undo;
      oldUndo!();
      expect(model.mail('2')!.folder, 'Trash');
      repo.jobs[0].completeError(StateError('Old rejection'));
      await first;
      expect(model.moves.label, 'Deleted 1 message');
      expect(model.undo, same(newUndo));
      model.moves.dismiss();
      repo.jobs[1].complete();
      await second;
      expect(model.moves.label, isNull);
      expect(model.undo, isNull);
    },
  );
}
