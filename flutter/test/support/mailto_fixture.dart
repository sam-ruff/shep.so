import 'dart:async';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/drafts.dart';
import 'package:shep_mobile/data/mailto_links.dart';
import 'package:shep_mobile/model/mail.dart';
import 'paged_repository.dart';

const mailtoAccount = MailAccount(
  id: 'fixture-account',
  name: 'Fixture account',
  email: 'alex@example.test',
  host: 'imap.example.test',
  port: 993,
  username: 'alex',
  smtpHost: 'smtp.example.test',
  smtpPort: 465,
);

/// Stands in for the platform channel; links are only delivered by [arrive]
/// or by being pending before the app starts.
class MailtoLinksFixture implements MailtoLinks {
  final pending = <String>[];
  final _arrivals = StreamController<void>.broadcast();
  int takes = 0;
  void arrive(String link) {
    pending.add(link);
    _arrivals.add(null);
  }

  @override
  Stream<void> get arrivals => _arrivals.stream;
  @override
  Future<List<String>> take() async {
    takes++;
    final links = List.of(pending);
    pending.clear();
    return links;
  }
}

typedef MailtoRequest = ({
  String id,
  String link,
  String account,
  bool message,
});

/// The shared Rust parser is exercised through actual FFI elsewhere; this
/// fixture returns the fields each test names for a link.
class MailtoRepositoryFixture extends PagedRepository
    implements MailtoRepository {
  MailtoRepositoryFixture({this.accounts = const [mailtoAccount]});
  List<MailAccount> accounts;
  final fields = <String, Draft>{};
  final requests = <MailtoRequest>[];
  final sent = <Draft>[];
  Object? failure;
  Completer<void>? hold;
  /// Behaves like an installed client, where sending needs an account.
  @override
  bool get preview => false;
  @override
  List<MailAccount> get mailAccounts => accounts;
  @override
  Future<void> send(Draft draft) async => sent.add(draft);
  @override
  Future<Draft> mailtoDraft(
    String draftId,
    String link, {
    required String accountId,
    required bool message,
  }) async {
    requests.add((
      id: draftId,
      link: link,
      account: accountId,
      message: message,
    ));
    await hold?.future;
    if (failure case final Object error) throw error;
    final template = fields[link] ?? const Draft(id: '');
    final draft = Draft(
      id: draftId,
      accountId: accountId,
      to: template.to,
      cc: message ? '' : template.cc,
      bcc: message ? '' : template.bcc,
      subject: message ? '' : template.subject,
      body: message ? '' : template.body,
    );
    drafts[draftId] = draft;
    return draft;
  }
}
