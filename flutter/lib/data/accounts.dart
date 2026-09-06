import '../model/mail.dart';

class MailAccount {
  const MailAccount({
    required this.id,
    required this.name,
    required this.email,
    required this.host,
    required this.port,
    required this.username,
    required this.smtpHost,
    required this.smtpPort,
    this.protocol = 'Imap',
    this.security = 'Tls',
    this.authentication = 'Password',
    this.smtpUsername = '',
    this.smtpSecurity = 'Tls',
    this.smtpAuthentication = 'Automatic',
    this.separatePassword = false,
    this.sentCopy = 'Automatic',
    this.sentFolder = '',
  });
  final String id,
      name,
      email,
      host,
      username,
      smtpHost,
      protocol,
      security,
      authentication,
      smtpUsername,
      smtpSecurity,
      smtpAuthentication,
      sentCopy,
      sentFolder;
  final int port, smtpPort;
  final bool separatePassword;
  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'email': email,
    'protocol': protocol,
    'host': host,
    'port': port,
    'username': username,
    'incoming_security': security,
    'incoming_auth': authentication,
    'smtp_host': smtpHost,
    'smtp_port': smtpPort,
    'smtp_username': smtpUsername,
    'smtp_security': smtpSecurity,
    'smtp_auth': smtpAuthentication,
    'smtp_separate_password': separatePassword,
    'sent_copy': sentCopy,
    'sent_folder': sentFolder,
  };
  factory MailAccount.fromJson(Map<String, dynamic> json) => MailAccount(
    id: json['id'],
    name: json['name'],
    email: json['email'],
    host: json['host'],
    port: json['port'],
    username: json['username'],
    smtpHost: json['smtp_host'],
    smtpPort: json['smtp_port'],
    protocol: json['protocol'],
    security: json['incoming_security'],
    authentication: json['incoming_auth'],
    smtpUsername: json['smtp_username'],
    smtpSecurity:
        json['smtp_security'] ??
        (json['smtp_port'] == 465 ? 'Tls' : 'StartTls'),
    smtpAuthentication: json['smtp_auth'],
    separatePassword: json['smtp_separate_password'],
    sentCopy: json['sent_copy'],
    sentFolder: json['sent_folder'],
  );
}

class MailPage {
  const MailPage(
    this.mail,
    this.total,
    this.unread, {
    this.aliases = const {},
    this.folderMembership = const {},
  });
  final Map<String, Set<String>> folderMembership;
  final Map<String, String> aliases;
  final List<Mail> mail;
  final int total, unread;
}

abstract interface class AccountRepository {
  List<MailAccount> get mailAccounts;
  Map<String, List<String>> get folderNames;
  List<Draft> get savedDrafts;
  String? get warning;
  Future<void> initialize();
  Future<void> connect(MailAccount account, String incoming, String smtp);
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
  });
  Future<Mail> detail(String id);
  Future<void> discard(String id, int revision);
  Future<String?> delivery(String id);
}

class MailOperationFailure implements Exception {
  const MailOperationFailure(
    this.message, {
    this.committed = false,
    this.refreshFirst = false,
  });
  final String message;
  final bool committed, refreshFirst;
  @override
  String toString() => message;
}

abstract interface class SentPreferencesRepository {
  Future<void> saveSentPreferences(
    String account,
    String policy,
    String folder,
  );
}

class AccountRemoval {
  const AccountRemoval(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'];
  String get email => data['email'];
  int count(String key) => data[key] as int;
  bool get unfinished => count('unresolved') > 0 || count('moves') > 0;
}

abstract interface class AccountRemovalRepository {
  int get pendingCredentialCleanup;
  Future<AccountRemoval> removalPreview(String id);
  Future<void> removeAccount(AccountRemoval review, bool discardUnresolved);
  Future<void> cleanupCredentials();
}
