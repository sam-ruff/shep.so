import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/groups.dart';
import 'package:shep_mobile/model/mail_groups.dart';
import 'package:shep_mobile/model/mail_selection.dart';
import 'support/bulk_fixture.dart';
import 'support/preview_repository.dart';

Future<void> settled(bool Function() ready) async {
  for (var i = 0; !ready(); i++) {
    if (i == 5000) fail('Controllers did not settle');
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
}

class Harness {
  Harness({Duration stepDelay = Duration.zero})
    : repository = bulkPreviewRepository(stepDelay: stepDelay) {
    selection = MailSelection(
      repository: repository,
      scope: () => {'folder': 'Inbox'},
      currentCount: () => inbox.length,
      changed: () => changes++,
    );
    groups = MailGroups(
      repository: repository,
      changed: () => changes++,
      refreshMail: () async => repaints++,
    );
  }
  final PreviewRepository repository;
  late final MailSelection selection;
  late final MailGroups groups;
  int changes = 0, repaints = 0;
  List<String> get inbox => repository.cached
      .where((m) => m.folder == 'Inbox')
      .map((m) => m.id)
      .toList();

  Future<GroupJob> reviewAll(GroupAction action) async {
    for (final id in inbox.take(50)) {
      selection.watch(id);
    }
    selection.all();
    await settled(() => selection.ready);
    final review = await groups.prepare(selection, action);
    expect(review, isNotNull);
    return review!;
  }

  Future<void> finished() =>
      settled(() => !groups.running && !groups.historyLoading);
  GroupJob job(String id) => groups.jobs.firstWhere((j) => j.id == id);
  void dispose() {
    selection.dispose();
    groups.dispose();
  }
}

void main() {
  test(
    'frozen review counts per account, approval executes every step',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.archive);
      expect(review.total, 130);
      expect(review.state, 'review');
      expect(
        review.groups.map(
          (g) => '${g['account']}:${g['folder']}:${g['total']}',
        ),
        containsAll(['Personal:Inbox:86', 'Work:Inbox:44']),
      );
      expect(h.selection.mode, false, reason: 'the review owns the membership');
      // Nothing paints or runs before approval.
      expect(h.inbox.length, 130);
      expect(await h.groups.approve(), true);
      expect(h.groups.review, isNull);
      expect(h.inbox, isEmpty, reason: 'approved intent paints immediately');
      await h.finished();
      final job = h.job(review.id);
      expect(job.finished, true);
      expect(job.count('done'), 130);
      expect(job.status, 'Archived 130');
      expect(
        h.repository.cached.where((m) => m.folder == 'Archive').length,
        131,
      );
      expect(h.groups.completed?.id, review.id);
      expect(h.repaints, greaterThan(0));
    },
  );

  test(
    'individual intent after approval wins and already-applied rows skip',
    () async {
      final h = Harness(stepDelay: const Duration(milliseconds: 5));
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.read);
      await h.groups.approve();
      // The newest per-field choice belongs to the individual action.
      unawaited(h.repository.mutate('bulk-124', {'unread': true}));
      await h.finished();
      final job = h.job(review.id);
      expect(job.count('done'), 64);
      expect(job.count('skipped'), 66);
      final items = await h.groups.items(job, after: 100);
      final newer = items.rows.firstWhere((i) => i.mail == 'bulk-124');
      expect(newer.state, 'skipped');
      expect(newer.reason, contains('newer change'));
      expect(
        h.repository.cached.firstWhere((m) => m.id == 'bulk-124').unread,
        true,
      );
    },
  );

  test(
    'pause holds the next step and resume continues the same group',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.flag);
      final hold = Completer<void>(), started = Completer<void>();
      h.repository.groupPreview.hold = hold;
      h.repository.groupPreview.stepStarted = started;
      await h.groups.approve();
      await started.future;
      expect(h.groups.running, true);
      await h.groups.pause(h.job(review.id));
      expect(h.job(review.id).paused, true);
      h.repository.groupPreview.hold = null;
      hold.complete();
      await h.finished();
      final paused = h.job(review.id);
      expect(paused.paused, true);
      expect(paused.count('done'), 1);
      expect(paused.count('pending'), 130 - 1 - paused.count('skipped'));
      await h.groups.resume(paused);
      await h.finished();
      expect(h.job(review.id).finished, true);
      expect(h.job(review.id).count('pending'), 0);
    },
  );

  test('undo cancels unsent steps and restores acknowledged rows', () async {
    final h = Harness();
    addTearDown(h.dispose);
    final review = await h.reviewAll(GroupAction.delete);
    final firstHold = Completer<void>(), firstStarted = Completer<void>();
    h.repository.groupPreview.hold = firstHold;
    h.repository.groupPreview.stepStarted = firstStarted;
    await h.groups.approve();
    await firstStarted.future;
    final secondHold = Completer<void>(), secondStarted = Completer<void>();
    h.repository.groupPreview.stepStarted = secondStarted;
    h.repository.groupPreview.hold = secondHold;
    firstHold.complete();
    await secondStarted.future;
    // The second step is in flight: Undo cancels the rest and reverses it
    // once its receipt lands.
    await h.groups.undo(h.job(review.id));
    final undoing = h.job(review.id);
    expect(undoing.undo, true);
    expect(undoing.state, 'undoing');
    expect(undoing.count('cancelled'), 128);
    h.repository.groupPreview.hold = null;
    secondHold.complete();
    await h.finished();
    final job = h.job(review.id);
    expect(job.finished, true);
    expect(job.count('undone'), 2);
    expect(job.count('cancelled'), 128);
    expect(h.inbox.length, 130);
    expect(job.status, 'Undone: 2 restored');
  });

  test(
    'failed steps retry explicitly, unconfirmed steps pause and are accepted',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      h.repository.groupPreview.outcomes['bulk-003'] = 'failed';
      h.repository.groupPreview.outcomes['bulk-010'] = 'uncertain';
      final review = await h.reviewAll(GroupAction.archive);
      await h.groups.approve();
      await h.finished();
      var job = h.job(review.id);
      expect(
        job.paused,
        true,
        reason: 'an unconfirmed result pauses the group',
      );
      expect(job.count('failed'), 1);
      expect(job.count('uncertain'), 1);
      expect(job.attention, 2);
      expect(h.groups.needingReview.length, 1);
      final page = await h.groups.items(job);
      final failed = page.rows.firstWhere((i) => i.state == 'failed');
      final uncertain = page.rows.firstWhere((i) => i.state == 'uncertain');
      expect(failed.canRetry, true);
      expect(uncertain.canAccept, true);
      expect(uncertain.canRetry, false);
      await h.groups.retry(job, failed);
      await h.finished();
      job = h.job(review.id);
      expect(job.count('failed'), 0);
      expect(job.paused, true);
      await h.groups.accept(job, uncertain);
      job = h.job(review.id);
      expect(job.count('accepted'), 1);
      await h.groups.resume(job);
      await h.finished();
      job = h.job(review.id);
      expect(job.finished, true);
      expect(job.count('done'), 129);
      expect(job.count('pending'), 0);
      expect(h.groups.error, isNull);
    },
  );

  test('a missing credential stops the loop with a retryable error', () async {
    final repository = _Unavailable();
    final groups = MailGroups(
      repository: repository,
      changed: () {},
      refreshMail: () async {},
    );
    addTearDown(groups.dispose);
    await groups.pump();
    expect(groups.running, false);
    expect(groups.error, contains('credential is unavailable'));
    expect(repository.steps, 1);
    await groups.pump();
    expect(repository.steps, 2);
  });

  test(
    'declined reviews leave History and repeated approval is refused',
    () async {
      final h = Harness();
      addTearDown(h.dispose);
      final review = await h.reviewAll(GroupAction.unflag);
      await h.groups.decline();
      expect(h.groups.review, isNull);
      await h.groups.refreshHistory();
      expect(h.groups.jobs.where((j) => j.id == review.id), isEmpty);
      expect(
        h.repository.groupPreview.calls,
        containsAllInOrder(['prepare', 'decline']),
      );
    },
  );
}

class _Unavailable implements GroupRepository {
  int steps = 0;
  @override
  Future<dynamic> groups(Map<String, Object?> command) async => {
    'jobs': const [],
    'runnable': false,
  };
  @override
  Future<Map<String, dynamic>> groupStep() async {
    steps++;
    return {'requires_credentials': 'fixture'};
  }
}
