import 'dart:convert';
import 'attachments.dart';
import 'package:flutter/foundation.dart';
import '../src/rust/api.dart';
import '../src/rust/frb_generated.dart';
import '../model/mail.dart';
import 'repository.dart';
import 'accounts.dart';
import 'credentials.dart';
import 'drafts.dart';
import 'outgoing.dart';

Map<String, dynamic> _decode(String data) =>
    jsonDecode(data) as Map<String, dynamic>;

class NativeRepository
    implements
        MailRepository,
        AccountRepository,
        DraftRepository,
        OutgoingRepository,
        SentPreferencesRepository,
        AttachmentRepository {
  NativeRepository(this.profile, this.credentials);
  final MobileProfile profile;
  final CredentialStore credentials;
  @override
  bool get preview => false;
  @override
  List<Mail> cached = [];
  @override
  List<CalendarEntry> events = [];
  @override
  List<MailAccount> mailAccounts = [];
  @override
  Map<String, List<String>> folderNames = {};
  @override
  List<Draft> savedDrafts = [];
  @override
  String? warning;
  static Future<NativeRepository> open(
    String path, {
    CredentialStore credentials = const DeviceCredentials(),
  }) async {
    if (!ShepNative.instance.initialized) await ShepNative.init();
    return NativeRepository(await MobileProfile.open(path: path), credentials);
  }

  Future<dynamic> call(Map<String, Object?> request) async {
    final response = await compute(
      _decode,
      await profile.request(json: jsonEncode(request)),
    );
    if (response['error'] case final String error) {
      throw MailOperationFailure(error);
    }
    return response['data'];
  }

  @override
  Future<void> initialize() async {
    final state = await call({'op': 'accounts'}) as Map<String, dynamic>;
    mailAccounts = (state['accounts'] as List)
        .map((a) => MailAccount.fromJson(a))
        .toList();
    folderNames = (state['folders'] as Map<String, dynamic>).map(
      (k, v) => MapEntry(k, (v as List).cast<String>()),
    );
    savedDrafts = (await call({'op': 'drafts'}) as List)
        .map((d) => Draft.fromJson(d))
        .toList();
  }

  @override
  Future<void> connect(
    MailAccount account,
    String incoming,
    String smtp,
  ) async {
    await call({
      'op': 'probe',
      'account': account.toJson(),
      'password': incoming,
      'smtp': false,
    });
    await call({
      'op': 'probe',
      'account': account.toJson(),
      'password': smtp,
      'smtp': true,
    });
    // No account-save success is reported before the platform credential store
    // has acknowledged both independent credentials.
    try {
      await credentials.save(account.id, incoming, smtp);
    } catch (_) {
      throw const MailOperationFailure(
        'The device could not save the passwords. Unlock its credential storage and retry connecting.',
      );
    }
    await call({
      'op': 'save_account',
      'account': account.toJson(),
      'preserve_sent': mailAccounts.any((a) => a.id == account.id),
    });
    await initialize();
  }

  Future<String> password(MailAccount account, {bool smtp = false}) async {
    try {
      final value = await credentials.read(account.id, smtp);
      if (value == null) {
        throw const MailOperationFailure(
          'The saved password is missing. Reconnect this account in Preferences.',
        );
      }
      return value;
    } on MailOperationFailure {
      rethrow;
    } catch (_) {
      throw const MailOperationFailure(
        'The device credential store is unavailable. Unlock the device and retry; cached mail is available.',
      );
    }
  }

  Mail mailFrom(
    Map<String, dynamic> m, {
    String body = '',
    List<String>? attachments,
    List<ReceivedAttachment> files = const [],
    bool? loaded,
    String? fileError,
  }) {
    final sender = (m['sender'] as String).trim().isEmpty
        ? 'Unknown sender'
        : m['sender'] as String;
    final address =
        RegExp(r'<([^<>]+)>').firstMatch(sender)?.group(1) ?? sender;
    return Mail(
      id: m['id'],
      sender: sender
          .replaceFirst(RegExp(r'\s*<[^<>]+>$'), '')
          .replaceAll('"', ''),
      address: address,
      subject: m['subject'],
      preview: m['preview'],
      body: body,
      date: DateTime.fromMillisecondsSinceEpoch((m['timestamp'] as int) * 1000),
      account:
          mailAccounts
              .where((a) => a.id == m['account_id'])
              .firstOrNull
              ?.email ??
          m['account_id'],
      accountId: m['account_id'],
      folder: m['folder'] == 'INBOX' ? 'Inbox' : m['folder'],
      unread: m['unread'],
      starred: m['starred'],
      attachments:
          attachments ??
          List.generate(
            m['attachment_count'] as int,
            (i) => 'Attachment ${i + 1}',
          ),
      files: files,
      fileError: fileError,
      bodyLoaded: loaded ?? body.isNotEmpty,
    );
  }

  @override
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
  }) async {
    final id = mailAccounts
        .where((a) => a.id == account || a.email == account)
        .firstOrNull
        ?.id;
    final data =
        await call({
              'op': 'page',
              'folder': folder,
              'account': id,
              'query': query,
              'filter': filter,
              'oldest': oldest,
              'offset': offset,
            })
            as Map<String, dynamic>;
    final mail = (data['mail'] as List).map((m) => mailFrom(m)).toList();
    cached = mail;
    return MailPage(
      mail,
      data['total'],
      data['unread'],
      aliases: (data['aliases'] as Map<String, dynamic>? ?? {})
          .cast<String, String>(),
      folderMembership:
          (data['folder_membership'] as Map<String, dynamic>? ?? {}).map(
            (account, names) => MapEntry(
              account,
              (names as List)
                  .cast<String>()
                  .map((name) => name == 'INBOX' ? 'Inbox' : name)
                  .toSet(),
            ),
          ),
    );
  }

  @override
  Future<Mail> detail(String id) async {
    final data = await call({'op': 'detail', 'id': id});
    return mailFrom(
      data['summary'],
      body: data['body'],
      attachments: (data['attachments'] as List).cast<String>(),
      files: (data['files'] as List? ?? [])
          .map((f) => ReceivedAttachment.fromJson(f))
          .toList(),
      loaded: true,
      fileError: data['file_error'],
    );
  }

  @override
  Future<Uint8List> attachment(String message, ReceivedAttachment file) async {
    final result = await call({
      'op': 'attachment',
      'id': message,
      'file': file.id,
    });
    final info = ReceivedAttachment.fromJson(result['info']);
    if (info.id != file.id || info.size != file.size) {
      throw const MailOperationFailure(
        'This attachment changed. Reopen the message and retry.',
      );
    }
    return compute(base64Decode, result['bytes'] as String);
  }

  @override
  Future<List<Mail>> refresh() async {
    warning = null;
    if (mailAccounts.isEmpty) {
      throw const MailOperationFailure(
        'Add a mail account in Preferences to refresh.',
      );
    }
    final errors = <String>[];
    for (final account in mailAccounts) {
      try {
        final result = await call({
          'op': 'sync',
          'account': account.id,
          'password': await password(account),
        });
        if ((result['skipped_large'] as int) > 0) {
          errors.add('Some mail exceeds the current 25 MiB download limit.');
        }
      } catch (error) {
        errors.add('$error');
      }
    }
    await initialize();
    warning = errors.isEmpty ? null : errors.join('\n');
    return cached;
  }

  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    final request = <String, Object?>{
      'op': 'mutate',
      'id': id,
      ...fields.map(
        (k, v) => MapEntry(k, k == 'folder' && v == 'Inbox' ? 'INBOX' : v),
      ),
    };
    // Rust decides from the current stored message, including a Sent copy that
    // has since synced. Purely local actions never open device credentials.
    var result = await call(request);
    if (result['requires_credentials'] case final String accountId) {
      final account = mailAccounts.where((a) => a.id == accountId).firstOrNull;
      if (account == null) {
        throw const MailOperationFailure(
          'This account changed. Reopen Preferences and refresh its folders.',
        );
      }
      result = await call({...request, 'password': await password(account)});
    }
    if (result['warning'] case final String message) {
      throw MailOperationFailure(
        message,
        committed: result['committed'] == true,
      );
    }
  }

  @override
  Future<DraftFiles> files(String id) async =>
      DraftFiles.fromJson(await call({'op': 'draft_files', 'id': id}));
  @override
  Future<DraftFiles> addFiles(
    String id,
    List<SelectedAttachment> selected,
  ) async => DraftFiles.fromJson(
    await call({
      'op': 'add_draft_files',
      'id': id,
      'paths': selected.map((f) => f.toJson()).toList(),
    }),
  );
  @override
  Future<DraftFiles> removeFile(String id, String file) async =>
      DraftFiles.fromJson(
        await call({'op': 'remove_draft_file', 'id': id, 'file': file}),
      );
  @override
  Future<Draft> reply(String id, bool all) async =>
      Draft.fromJson(await call({'op': 'reply', 'id': id, 'all': all}));
  @override
  Future<void> saveDraft(Draft draft) async {
    await call({'op': 'save_draft', 'draft': draft.toJson()});
  }

  @override
  Future<void> discard(String id, int revision) async {
    await call({'op': 'discard_draft', 'id': id, 'revision': revision});
  }

  @override
  Future<OutgoingPage> outbox({int offset = 0}) async =>
      OutgoingPage.fromJson(await call({'op': 'outbox', 'offset': offset}));
  @override
  Future<OutgoingResult> recoverOutgoing(
    String id,
    OutgoingAction action, {
    bool confirmed = false,
  }) async {
    final remote =
        action == OutgoingAction.checkSent || action == OutgoingAction.copySent;
    String? incoming;
    if (remote) {
      final context = await call({'op': 'outgoing_account', 'id': id});
      final account = mailAccounts
          .where((a) => a.id == context['account_id'])
          .firstOrNull;
      if (account == null) {
        throw const MailOperationFailure(
          'Reconnect the original account before checking Sent.',
        );
      }
      // A known provider acknowledgment can be persisted without credentials.
      // A missing credential remains a recoverable provider error in Outbox.
      try {
        incoming = await password(account);
      } on MailOperationFailure {
        incoming = null;
      }
    }
    final result = OutgoingResult.fromJson(
      await call(
        remote
            ? {
                'op': 'sent_outgoing',
                'id': id,
                'copy': action == OutgoingAction.copySent,
                'confirmed': confirmed,
                'password': incoming,
              }
            : {
                'op': 'recover_outgoing',
                'id': id,
                'action': action == OutgoingAction.backToDrafts
                    ? 'return'
                    : action.name,
                'confirmed': confirmed,
              },
      ),
    );
    await initialize();
    return result;
  }

  @override
  Future<void> saveSentPreferences(
    String account,
    String policy,
    String folder,
  ) async {
    await call({
      'op': 'save_sent_preferences',
      'id': account,
      'policy': policy,
      'folder': folder,
    });
    await initialize();
  }

  @override
  Future<String?> delivery(String id) async {
    final result = await call({'op': 'delivery', 'id': id});
    return result?['state'];
  }

  @override
  Future<void> send(Draft draft) async {
    final prior = await delivery(draft.id);
    if (prior == 'delivered') return;
    if (prior != null) {
      throw MailOperationFailure(
        'Delivery is $prior. Check Sent or the recipient before composing another copy. This draft was not resent.',
      );
    }
    final account = mailAccounts
        .where((a) => a.id == draft.accountId)
        .firstOrNull;
    if (account == null) {
      throw const MailOperationFailure('Choose a sending account.');
    }
    await saveDraft(draft);
    String? incoming;
    if (account.protocol == 'Imap' && account.sentCopy != 'LocalOnly') {
      try {
        incoming = await password(account);
      } on MailOperationFailure {
        incoming = null;
      }
    }
    final result = await call({
      'op': 'send',
      'incoming_password': incoming,
      'revision': draft.revision,
      'file_revision': draft.fileRevision,
      'id': draft.id,
      'password': await password(account, smtp: true),
    });
    if (result['warning'] case final String message) {
      throw MailOperationFailure(message);
    }
    if (result['state'] != 'delivered') {
      throw MailOperationFailure(
        'Delivery is ${result['state']}. Your draft and delivery record were kept. Check Sent before another send.',
      );
    }
  }

  @override
  Future<void> saveEvent(
    CalendarEntry event,
  ) async => throw const MailOperationFailure(
    'Calendar providers have not been connected yet. Your event remains open.',
  );
}
