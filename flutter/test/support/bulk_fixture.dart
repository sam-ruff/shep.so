import 'package:shep_mobile/model/mail.dart';
import 'preview_repository.dart';

/// 125 fictional Inbox messages across two accounts, older than the base
/// preview fixture so the fixed rows stay first. Three pages of 50 exercise
/// cross-page membership without loading the whole group into Dart.
List<Mail> bulkFixtureMail([int count = 125]) => List.generate(
  count,
  (i) => Mail(
    id: 'bulk-${i.toString().padLeft(3, '0')}',
    sender: i.isEven ? 'Robin Field' : 'Sam Lane',
    address: i.isEven ? 'robin@example.test' : 'sam@example.test',
    subject: 'Bulk message ${i + 1}',
    preview: 'Fictional message ${i + 1} for group action previews.',
    body: 'Fictional body ${i + 1}.',
    account: i % 3 == 0 ? 'Work' : 'Personal',
    folder: 'Inbox',
    date: DateTime.utc(2026, 8, 1).subtract(Duration(minutes: i)),
    unread: i % 2 == 0,
    starred: i % 5 == 0,
    attachments: const [],
  ),
);

PreviewRepository bulkPreviewRepository({
  Duration delay = Duration.zero,
  Duration stepDelay = Duration.zero,
}) =>
    PreviewRepository(delay: delay, extra: bulkFixtureMail())
      ..stepDelay = stepDelay;
