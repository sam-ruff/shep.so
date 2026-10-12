import 'accounts.dart';

typedef IncomingIdentity = ({
  String protocol,
  String host,
  int port,
  String username,
  String security,
  String authentication,
  String slot,
});

IncomingIdentity incomingIdentity(MailAccount account, String slot) => (
  protocol: account.protocol,
  host: account.host,
  port: account.port,
  username: account.username,
  security: account.security,
  authentication: account.authentication,
  slot: slot,
);

class IncomingAccount {
  const IncomingAccount(this.id, this.name, this.identity);
  final String id, name;
  final IncomingIdentity identity;
}

class IncomingResult {
  const IncomingResult(this.account, {this.error});
  final IncomingAccount account;
  final String? error;
}

abstract interface class IncomingSyncRepository {
  List<IncomingAccount> get incomingAccounts;
  Future<List<IncomingResult>> refreshIncoming({
    bool Function()? canDispatch,
    void Function(IncomingResult)? onResult,
  });
}
