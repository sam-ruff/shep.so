import 'dart:async';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'preview_repository.dart';

// Test-only paged provider. Mutation barriers are transport fixtures; every
// integration action still enters through the actual Flutter controls.
class PagedRepository extends PreviewRepository implements AccountRepository {
  PagedRepository() : super(delay: Duration.zero);
  final jobs = <Completer<void>>[];
  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    final job = Completer<void>();
    jobs.add(job);
    await job.future;
    await super.mutate(id, fields);
  }

  @override
  List<MailAccount> get mailAccounts => [];
  @override
  Map<String, List<String>> get folderNames => {};
  @override
  List<Draft> get savedDrafts => [];
  @override
  String? get warning => null;
  @override
  Future<void> initialize() async {}
  @override
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
  }) async {
    final rows = cached.where((m) => m.folder == folder).toList();
    return MailPage(
      rows.skip(offset).take(50).map((m) => m.withoutBody()).toList(),
      rows.length,
      cached.where((m) => m.folder == 'Inbox' && m.unread).length,
    );
  }

  @override
  Future<Mail> detail(String id) async => cached.firstWhere((m) => m.id == id);
  @override
  Future<void> connect(
    MailAccount account,
    String incoming,
    String smtp,
  ) async => throw UnimplementedError();
  @override
  Future<void> discard(String id, int revision) async =>
      throw UnimplementedError();
  @override
  Future<String?> delivery(String id) async => null;
}
