import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/mail_groups.dart';
import 'support/history_repository.dart';
import 'mail_selection_test.dart' show settled;

void main() {
  for (final command in ['undo', 'resume', 'retry']) {
    test(
      '$command during final History inspection preserves its pump wake',
      () async {
        final repository = HistoryRepository(groups: 1, items: 1);
        repository.records['group-000']!['state'] = command == 'resume'
            ? 'paused'
            : 'finished';
        repository.members['group-000']![0]['state'] = command == 'undo'
            ? 'done'
            : command == 'resume'
            ? 'pending'
            : 'failed';
        final groups = MailGroups(
          repository: repository,
          changed: () {},
          refreshMail: () async {},
        );
        addTearDown(groups.dispose);
        await groups.refreshHistory();
        final job = groups.jobs.single;
        final item = (await groups.items(job)).rows.single;
        repository.ownerHold = Completer<void>();
        final pumping = groups.pump();
        await settled(() => groups.historyLoading);
        expect(repository.stepCalls, 1);
        switch (command) {
          case 'undo':
            await groups.undo(job);
          case 'resume':
            await groups.resume(job);
          case 'retry':
            await groups.retry(job, item);
        }
        repository.ownerHold!.complete();
        await pumping;
        await settled(() => !groups.running && !groups.historyLoading);
        expect(repository.stepCalls, 2);
      },
    );
  }
}
