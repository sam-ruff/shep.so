import 'selection.dart';
import 'groups.dart';
import 'folders.dart';
import 'dart:async';
import 'dart:convert';
import 'attachments.dart';
import 'message_search.dart';
import 'formatted_message.dart';
import 'printing.dart';
import 'package:flutter/foundation.dart';
import '../src/rust/api.dart';
import '../src/rust/frb_generated.dart';
import '../model/mail.dart';
import 'repository.dart';
import 'accounts.dart';
import 'credentials.dart';
import 'drafts.dart';
import 'outgoing.dart';
import 'profile_enrollment.dart';

Map<String, dynamic> _decode(String data) =>
    jsonDecode(data) as Map<String, dynamic>;
String _encode(Map<String, Object?> data) => jsonEncode(data);

class NativeRepository
    implements
        SelectionRepository,
        GroupRepository,
        MailRepository,
        AccountRepository,
        ProfileAccountRepository,
        DraftRepository,
        ForwardRepository,
        OutgoingRepository,
        SentPreferencesRepository,
        AttachmentRepository,
        AccountRemovalRepository,
        TextSearchRepository,
        FormattedMessageRepository,
        PrintRepository,
        MailActivityRepository,
        DurableAccountRepository,
        DurableCalendarRepository,
        DurableCalDavRepository,
        FolderChangeRepository,
        DurableMutationRepository {
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

  Future<dynamic> call(Map<String, Object?> request) async =>
      _send(jsonEncode(request));

  /// Profile records may contain substantial metadata. Encode them outside the
  /// UI isolate; their controller must still serialize dependent requests.
  Future<dynamic> callBackground(Map<String, Object?> request) async =>
      _send(await compute(_encode, request));

  Future<dynamic> _send(String encoded) async {
    final response = await compute(
      _decode,
      await profile.request(json: encoded),
    );
    if (response['error'] case final String error) {
      throw MailOperationFailure(error);
    }
    return response['data'];
  }

  @override
  Future<PreparedPrint> preparePrint(
    String id, {
    required String generation,
    required bool plain,
  }) async => PreparedPrint.fromJson(
    await call({
      'op': 'print',
      'id': id,
      'options': {'generation': generation, 'plain': plain},
    }),
  );

  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) async => PreparedMessage.fromJson(
    await call({
          'op': 'formatted',
          'id': id,
          'options': {'generation': generation, 'dark': dark, 'quotes': quotes},
        })
        as Map<String, dynamic>,
  );

  @override
  Future<List<SearchHit>> findText(
    List<String> blocks,
    String query,
    bool matchCase,
  ) async =>
      (await call({
                'op': 'find_text',
                'blocks': blocks,
                'query': query,
                'match_case': matchCase,
              })
              as List)
          .map((h) => SearchHit.fromJson(h))
          .toList();

  @override
  Set<String> reconnectAccounts = {};
  @override
  List<AccountConnectionAttempt> connectionAttempts = const [];
  @override
  Future<void> refreshProfileAccounts() async {
    final state = await call({'op': 'accounts'}) as Map<String, dynamic>;
    mailAccounts = (state['accounts'] as List)
        .map((a) => MailAccount.fromJson(a))
        .toList();
    folderNames = (state['folders'] as Map<String, dynamic>).map(
      (k, v) => MapEntry(k, (v as List).cast<String>()),
    );
    reconnectAccounts = (state['reconnect'] as List? ?? const [])
        .cast<String>()
        .toSet();
  }

  @override
  Future<void> initialize() async {
    await refreshProfileAccounts();
    await refreshConnectionAttempts();
    pendingCredentialCleanup =
        (await call({'op': 'credential_cleanup'}) as List).length;
    savedDrafts = (await call({'op': 'drafts'}) as List)
        .map((d) => Draft.fromJson(d))
        .toList();
    unawaited(
      _resumeQueuedOutgoing().catchError((Object _) {
        warning =
            'Queued delivery could not resume. Open Outbox to check its status.';
      }),
    );
  }

  // Account/credential lifecycle operations share one FIFO across bridge
  // handles. The native profile lock excludes independent app processes.
  static Future<void> _accountWrites = Future.value();
  static final Map<String, Future<void>> _connectionExecutions = {};
  Future<T> _accountWrite<T>(Future<T> Function() operation) {
    final result = _accountWrites.then((_) => operation());
    _accountWrites = result.then<void>(
      (_) {},
      onError: (Object _, StackTrace _) {},
    );
    return result;
  }

  Future<dynamic> profileEnrollment(Map<String, Object?> command) =>
      _accountWrite(() => callBackground(command));

  @override
  int pendingCredentialCleanup = 0;
  @override
  Future<AccountRemoval> removalPreview(String id) async => AccountRemoval(
    Map<String, dynamic>.from(
      await call({'op': 'account_removal_preview', 'id': id}),
    ),
  );
  Future<void> _cleanupCredentials() async {
    // This query checks every current owner before deletion. The lifecycle
    // FIFO remains held until the OS operation and acknowledgment finish.
    final ids = (await call({'op': 'credential_cleanup'}) as List)
        .cast<String>();
    pendingCredentialCleanup = ids.length;
    for (final id in ids) {
      try {
        await credentials.remove(id);
        await call({'op': 'credential_cleanup_done', 'id': id});
        pendingCredentialCleanup--;
      } catch (_) {
        // Removal is already committed. Retain the durable cleanup job.
      }
    }
  }

  @override
  Future<void> cleanupCredentials() => _accountWrite(_cleanupCredentials);
  @override
  Future<void> removeAccount(AccountRemoval review, bool discardUnresolved) =>
      _accountWrite(() async {
        await call({
          'op': 'remove_account',
          'review': review.data,
          'discard_unresolved': discardUnresolved,
        });
        mailAccounts.removeWhere((a) => a.id == review.id);
        folderNames.remove(review.id);
        savedDrafts.removeWhere((d) => d.accountId == review.id);
        cached.removeWhere((m) => m.accountId == review.id);
        connectionAttempts = connectionAttempts
            .where((attempt) => attempt.account.id != review.id)
            .toList(growable: false);
        pendingCredentialCleanup++;
        try {
          await _cleanupCredentials();
        } catch (_) {
          /* durable job retries in Preferences */
        }
      });
  @override
  Future<void> connect(MailAccount account, String incoming, String smtp) =>
      () async {
        final attempt = await admitConnection(
          newConnectionAttemptId(),
          account,
        );
        await executeConnection(attempt, incoming, smtp);
      }();

  @override
  Future<AccountConnectionAttempt> admitConnection(
    String attempt,
    MailAccount account,
  ) async {
    dynamic prepared;
    try {
      prepared = await call({
        'op': 'prepare_account',
        'attempt': attempt,
        'account': account.toJson(),
        'expected': mailAccounts
            .where((a) => a.id == account.id)
            .firstOrNull
            ?.toJson(),
      });
    } catch (_) {
      await refreshConnectionAttempts();
      final saved = connectionAttempts
          .where((item) => item.id == attempt && item.account.id == account.id)
          .firstOrNull;
      if (saved == null) rethrow;
      return saved;
    }
    final admitted = AccountConnectionAttempt(
      id: prepared['attempt'] as String,
      account: MailAccount.fromJson(prepared['account']),
      status: 'saving',
    );
    connectionAttempts = [
      admitted,
      ...connectionAttempts.where((item) => item.account.id != account.id),
    ];
    return admitted;
  }

  @override
  Future<void> refreshConnectionAttempts() async {
    connectionAttempts =
        (await call({'op': 'pending_account_connections'}) as List)
            .map((value) {
              final row = Map<String, dynamic>.from(value);
              return AccountConnectionAttempt(
                id: row['attempt'] as String,
                account: MailAccount.fromJson(
                  Map<String, dynamic>.from(row['account']),
                ),
                status: row['status'] as String,
                error: row['error'] as String?,
              );
            })
            .toList(growable: false);
  }

  @override
  Future<void> executeConnection(
    AccountConnectionAttempt attempt,
    String incoming,
    String smtp, {
    bool Function()? canDispatch,
  }) async {
    if (_connectionExecutions[attempt.id] case final Future<void> running) {
      return running;
    }
    late final Future<void> execution;
    execution = _executeConnection(attempt, incoming, smtp, canDispatch)
        .whenComplete(() {
          if (identical(_connectionExecutions[attempt.id], execution)) {
            _connectionExecutions.remove(attempt.id);
          }
        });
    _connectionExecutions[attempt.id] = execution;
    return execution;
  }

  Future<void> _executeConnection(
    AccountConnectionAttempt attempt,
    String incoming,
    String smtp,
    bool Function()? canDispatch,
  ) async {
    try {
      await _accountWrite(() async {
        _requireDispatch(canDispatch);
        await call({'op': 'retry_account_connection', 'attempt': attempt.id});
      });
      await _connect(attempt, incoming, smtp, canDispatch);
    } catch (_) {
      await refreshConnectionAttempts();
      try {
        final target = await call({
          'op': 'credential_target',
          'account': attempt.account.toJson(),
        });
        if (target['slot'] == 'credential-${attempt.id}') {
          await refreshProfileAccounts();
          return;
        }
      } catch (_) {
        // The original connection failure remains authoritative.
      }
      rethrow;
    }
  }

  @override
  Future<void> failConnection(String attempt, String error) => call({
    'op': 'fail_account_connection',
    'attempt': attempt,
    'error': error,
  });

  Future<void> _connect(
    AccountConnectionAttempt attempt,
    String incoming,
    String smtp,
    bool Function()? canDispatch,
  ) async {
    final slot = 'credential-${attempt.id}';
    final savedAccount = attempt.account;
    try {
      for (final outgoing in [false, true]) {
        _requireDispatch(canDispatch);
        await call({
          'op': 'validate_account_connection',
          'attempt': attempt.id,
        });
        _requireDispatch(canDispatch);
        await call({
          'op': 'probe_account_connection',
          'attempt': attempt.id,
          'password': outgoing ? smtp : incoming,
          'smtp': outgoing,
        });
      }
      await _accountWrite(() async {
        _requireDispatch(canDispatch);
        await call({
          'op': 'validate_account_connection',
          'attempt': attempt.id,
        });
        _requireDispatch(canDispatch);
        try {
          await credentials.save(slot, incoming, smtp);
        } catch (_) {
          throw const MailOperationFailure(
            'The device could not save the passwords. Unlock its credential storage and retry connecting. The previous connection is preserved.',
          );
        }
        // The previous pair is untouched until this database pointer commits.
        _requireDispatch(canDispatch);
        await call({
          'op': 'validate_account_connection',
          'attempt': attempt.id,
        });
        _requireDispatch(canDispatch);
        await call({'op': 'activate_account', 'slot': slot});
      });
    } finally {
      // An activated slot is excluded by the durable journal. A failed probe,
      // keychain save, activation or lost response never erases the active pair.
      try {
        await _accountWrite(_cleanupCredentials);
      } catch (_) {
        /* Retry in Preferences. */
      }
    }
    connectionAttempts = connectionAttempts
        .where((item) => item.id != attempt.id)
        .toList(growable: false);
    mailAccounts = [
      ...mailAccounts.where((a) => a.id != savedAccount.id),
      savedAccount,
    ];
    try {
      await initialize();
    } catch (_) {
      throw const MailOperationFailure(
        'The connection was saved, but the account list could not reload. Reopen Preferences or refresh mail.',
      );
    }
  }

  void _requireDispatch(bool Function()? canDispatch) {
    if (canDispatch?.call() == false) {
      throw const MailOperationFailure(
        'Connection paused. Reopen the app and retry connecting.',
      );
    }
  }

  @override
  Future<void> abandonConnection(String attempt) async {
    await call({'op': 'abandon_account_connection', 'attempt': attempt});
    connectionAttempts = connectionAttempts
        .where((item) => item.id != attempt)
        .toList(growable: false);
    unawaited(
      _accountWrite(_cleanupCredentials).catchError((Object _) {
        warning =
            'A saved password cleanup is waiting. Retry it in Preferences.';
      }),
    );
  }

  Future<String> _readPassword(String slot, {bool smtp = false}) async {
    try {
      final value = await credentials.read(slot, smtp);
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

  Future<String> _credentialSlot(MailAccount account) async =>
      (await call({
            'op': 'credential_target',
            'account': account.toJson(),
          }))['slot']
          as String;

  Future<String> password(MailAccount account, {bool smtp = false}) async =>
      _readPassword(await _credentialSlot(account), smtp: smtp);

  Future<Map<String, Object?>> _incoming(MailAccount account) async {
    final slot = await _credentialSlot(account);
    return {'credential_slot': slot, 'password': await _readPassword(slot)};
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
      lineage: m['lineage'] as String?,
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
  Future<dynamic> selection(
    Map<String, Object?> command, {
    List<String> observed = const [],
  }) async {
    final value = Map<String, Object?>.of(command);
    if (value['scope'] case final Map original) {
      final scope = Map<String, Object?>.from(original);
      final selected = scope['account'];
      if (selected != null) {
        final account = mailAccounts
            .where((a) => a.id == selected || a.email == selected)
            .firstOrNull;
        if (account == null) {
          throw const MailOperationFailure(
            'This account was removed. Select messages from a connected account.',
          );
        }
        scope['account'] = account.id;
      }
      value['scope'] = scope;
    }
    return call({'op': 'selection', 'command': value, 'observed': observed});
  }

  Map<String, Object?> _groupScope(Map<String, Object?> command) {
    final value = Map<String, Object?>.of(command);
    if (value['scope'] case final Map original) {
      final scope = Map<String, Object?>.from(original);
      if (scope['account'] case final Object selected) {
        scope['account'] = mailAccounts
            .where((a) => a.id == selected || a.email == selected)
            .firstOrNull
            ?.id;
      }
      value['scope'] = scope;
    }
    return value;
  }

  @override
  Future<dynamic> groups(Map<String, Object?> command) =>
      call({'op': 'groups', 'command': _groupScope(command)});

  @override
  Future<Map<String, dynamic>> groupStep() async {
    var result =
        await call({
              'op': 'groups',
              'command': {'kind': 'step'},
            })
            as Map<String, dynamic>;
    if (result['requires_credentials'] case final String accountId) {
      final account = mailAccounts.where((a) => a.id == accountId).firstOrNull;
      if (account == null) {
        throw const MailOperationFailure(
          'This account changed. Reopen Preferences and refresh its folders.',
        );
      }
      // The credential enters only this one step; Rust never stores it.
      result =
          await call({
                'op': 'groups',
                'command': {'kind': 'step', ...await _incoming(account)},
              })
              as Map<String, dynamic>;
    }
    return result;
  }

  @override
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
    Map<String, Map<String, Object>> projection = const {},
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
              'projection': projection,
            })
            as Map<String, dynamic>;
    final mail = (data['mail'] as List).map((m) => mailFrom(m)).toList();
    cached = mail;
    return MailPage(
      mail,
      data['total'],
      data['unread'],
      confirmed: {
        for (final m in data['confirmed'] as List? ?? [])
          m['id'] as String: mailFrom(m),
      },
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
          ...await _incoming(account),
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
    await _mutate(id, fields, newDraftIdentity());
  }

  Map<String, Object?> _mutationRequest(
    String id,
    Map<String, Object> fields,
    String actionId, {
    String? observedLineage,
    bool requireObservation = false,
  }) => <String, Object?>{
    'op': 'mutate',
    'action_id': actionId,
    'observed_lineage': ?observedLineage,
    if (requireObservation) 'require_observation': true,
    'id': id,
    ...fields.map(
      (k, v) => MapEntry(k, k == 'folder' && v == 'Inbox' ? 'INBOX' : v),
    ),
  };

  @override
  Future<void> admitMutation(
    String id,
    Map<String, Object> fields,
    String actionId,
    String observedLineage,
  ) async {
    final result = await call(
      _mutationRequest(
        id,
        fields,
        actionId,
        observedLineage: observedLineage,
        requireObservation: true,
      ),
    );
    if (result['warning'] case final String message) {
      throw MailOperationFailure(
        message,
        committed: result['committed'] == true,
      );
    }
  }

  @override
  Future<void> executeMutation(
    String id,
    Map<String, Object> fields,
    String actionId,
  ) => _mutate(id, fields, actionId);

  @override
  Future<void> cancelAdmittedMutation(String actionId) =>
      cancelMailAction(actionId);

  Future<void> _mutate(
    String id,
    Map<String, Object> fields,
    String actionId,
  ) async {
    final request = _mutationRequest(id, fields, actionId);
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
      result = await call({...request, ...await _incoming(account)});
    }
    if (result['warning'] case final String message) {
      throw MailOperationFailure(
        message,
        committed: result['committed'] == true,
      );
    }
    if (result['status'] == 'cancelled') {
      throw const MailOperationFailure(
        'A newer mail decision replaced this queued change.',
      );
    }
  }

  @override
  Future<List<MailActivity>> mailActions({int offset = 0}) async =>
      ((await call({'op': 'mail_actions', 'offset': offset}))['actions']
              as List)
          .cast<Map<String, dynamic>>()
          .map(MailActivity.new)
          .toList();

  @override
  Future<List<MailActivity>> runnableMailActions({
    int? afterCreated,
    String? afterId,
  }) async =>
      ((await call({
                'op': 'mail_actions',
                'runnable': true,
                'after_created': ?afterCreated,
                'after_id': ?afterId,
              }))['actions']
              as List)
          .cast<Map<String, dynamic>>()
          .map(MailActivity.new)
          .toList();

  @override
  Future<void> resumeMailAction(MailActivity action) async =>
      _mutate(action.mail, action.fields, action.id);

  @override
  Future<List<FolderAccount>> folderOptions() async =>
      (await call({'op': 'folder_options'}) as List)
          .map(
            (value) => FolderAccount(Map<String, dynamic>.from(value as Map)),
          )
          .toList();

  @override
  Future<Map<String, dynamic>> reviewFolderChange(
    FolderAccount account,
    String source,
    Object action,
  ) async => Map<String, dynamic>.from(
    await call({
          'op': 'review_folder_change',
          'account': account.id,
          'source': source,
          'action': action,
        })
        as Map,
  );

  @override
  Future<FolderCreation> admitFolderChange(
    String id,
    Map<String, dynamic> review,
  ) async => FolderCreation(
    Map<String, dynamic>.from(
      await call({'op': 'admit_folder_change', 'id': id, 'review': review})
          as Map,
    ),
  );

  @override
  Future<List<FolderCreation>> folderCreations() async =>
      (await call({'op': 'folder_creations'}) as List)
          .map(
            (value) => FolderCreation(Map<String, dynamic>.from(value as Map)),
          )
          .toList();

  @override
  Future<FolderCreation> admitFolder(
    String id,
    FolderAccount account,
    String? parent,
    String name,
  ) async => FolderCreation(
    Map<String, dynamic>.from(
      await call({
            'op': 'admit_folder',
            'id': id,
            'account': account.id,
            'connection': account.connection,
            'parent': parent,
            'name': name,
          })
          as Map,
    ),
  );

  @override
  Future<FolderCreation> executeFolder(FolderCreation request) async {
    final account = mailAccounts
        .where((account) => account.id == request.account)
        .firstOrNull;
    Map<String, Object?> credentials = {};
    if (request.mutation?['prepared'] != false &&
        (!request.hasReceipt ||
            request.mutation != null && request.status == 'checking') &&
        account?.protocol != 'Pop3') {
      try {
        if (account == null) throw StateError('Account unavailable');
        credentials = await _incoming(account);
      } catch (_) {
        await call({
          'op': 'wait_folder',
          'id': request.id,
          'revision': request.revision,
        });
        rethrow;
      }
    }
    final result = FolderCreation(
      Map<String, dynamic>.from(
        await callBackground({
              'op': 'execute_folder',
              'id': request.id,
              ...credentials,
            })
            as Map,
      ),
    );
    if (result.status == 'succeeded') await refreshProfileAccounts();
    return result;
  }

  @override
  Future<FolderCreation> decideFolder(
    FolderCreation request,
    String decision,
  ) async => FolderCreation(
    Map<String, dynamic>.from(
      await call({
            'op': 'decide_folder',
            'id': request.id,
            'revision': request.revision,
            'decision': decision,
          })
          as Map,
    ),
  );

  @override
  Future<void> cancelMailAction(String id) async {
    await call({'op': 'cancel_mail_action', 'id': id});
  }

  @override
  Future<void> undoMailAction(
    MailActivity action, {
    void Function()? onAdmitted,
  }) async {
    final request = <String, Object?>{
      'op': 'undo_mail_action',
      'id': action.id,
    };
    var result = await call(request);
    onAdmitted?.call();
    if (result['requires_credentials'] case final String accountId) {
      final account = mailAccounts.where((a) => a.id == accountId).firstOrNull;
      if (account == null) {
        throw const MailOperationFailure(
          'This account changed. Reopen Preferences and refresh its folders.',
        );
      }
      result = await call({...request, ...await _incoming(account)});
    }
    if (result['warning'] case final String message) {
      throw MailOperationFailure(
        message,
        committed: result['committed'] == true,
      );
    }
  }

  @override
  Future<void> inspectMailAction(MailActivity action) async {
    final request = <String, Object?>{
      'op': 'inspect_mail_action',
      'id': action.id,
    };
    var result = await call(request);
    if (result['requires_credentials'] case final String accountId) {
      final account = mailAccounts.where((a) => a.id == accountId).firstOrNull;
      if (account == null) {
        throw const MailOperationFailure(
          'This account changed. Reopen Preferences and refresh its folders.',
        );
      }
      result = await call({...request, ...await _incoming(account)});
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
  Future<Draft> forward(String id, String draftId) async {
    final draft = Draft.fromJson(
      await call({'op': 'forward', 'id': id, 'draft_id': draftId}),
    );
    savedDrafts.removeWhere((d) => d.id == draft.id);
    savedDrafts.add(draft);
    return draft;
  }

  @override
  Future<Draft> reply(String id, bool all) async =>
      Draft.fromJson(await call({'op': 'reply', 'id': id, 'all': all}));
  @override
  Future<void> saveDraft(Draft draft) async {
    final text = draft.toJson()
      ..remove('forward')
      ..remove('attachments');
    await call({'op': 'save_draft', 'draft': text});
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
    Map<String, Object?> incoming = {};
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
        incoming = await _incoming(account);
      } on MailOperationFailure {
        incoming = {};
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
                ...incoming,
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
  Future<void> cancelOutgoing(String id) async {
    await call({'op': 'cancel_outgoing', 'id': id});
  }

  @override
  Future<void> resumeOutgoing(String id) async {
    final context = await call({'op': 'outgoing_account', 'id': id});
    final account = mailAccounts
        .where((candidate) => candidate.id == context['account_id'])
        .firstOrNull;
    if (account == null) {
      throw const MailOperationFailure(
        'Reconnect the original account before resuming delivery.',
      );
    }
    unawaited(_runQueuedSend(id, account));
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
    final attempt = newDraftIdentity();
    final admitted = await call({
      'op': 'admit_send',
      'attempt': attempt,
      'revision': draft.revision,
      'file_revision': draft.fileRevision,
      'id': draft.id,
    });
    if (admitted['state'] == 'delivered') return;
    if (!const {'queued', 'waiting'}.contains(admitted['state'])) {
      throw MailOperationFailure(
        'Delivery is ${admitted['state']}. Check Outbox before composing another copy.',
      );
    }
    unawaited(_runQueuedSend(admitted['id'] as String, account));
  }

  Future<void> _runQueuedSend(String attempt, MailAccount account) async {
    try {
      await _executeSend(attempt, account);
    } catch (_) {
      warning =
          'A queued delivery needs attention. Open Outbox to check its status.';
      try {
        await call({'op': 'wait_outgoing', 'id': attempt});
      } catch (_) {
        warning =
            'Delivery status could not be saved. Open Outbox before sending again.';
      }
    }
  }

  Future<void> _executeSend(String attempt, MailAccount account) async {
    final slot = await _credentialSlot(account);
    String? incoming;
    if (account.protocol == 'Imap' && account.sentCopy != 'LocalOnly') {
      try {
        incoming = await _readPassword(slot);
      } on MailOperationFailure {
        incoming = null;
      }
    }
    final result = await call({
      'op': 'send',
      'attempt': attempt,
      'incoming_password': incoming,
      'credential_slot': slot,
      'password': account.smtpAuthentication == 'None'
          ? ''
          : await _readPassword(slot, smtp: true),
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

  Future<void> _resumeQueuedOutgoing() async {
    final rows = ((await call({'op': 'runnable_outgoing'}))['rows'] as List)
        .cast<Map<String, dynamic>>();
    for (final row in rows) {
      final account = mailAccounts
          .where((candidate) => candidate.id == row['account_id'])
          .firstOrNull;
      if (account == null) continue;
      unawaited(_runQueuedSend(row['id'] as String, account));
    }
  }

  @override
  Future<void> saveEvent(
    CalendarEntry event,
  ) async => throw const MailOperationFailure(
    'Calendar providers have not been connected yet. Your event remains open.',
  );

  @override
  Future<CalDavAttempt> admitCalDavConnection({
    required String attemptId,
    required String connectionId,
    required String url,
    required String username,
    CalDavConnection? observed,
  }) async => CalDavAttempt(
    Map<String, dynamic>.from(
      await call({
            'op': 'prepare_calendar_connection',
            'id': attemptId,
            'request': {
              'connection': {
                'id': connectionId,
                'url': url,
                'username': username,
              },
              'credential_slot': 'calendar-$attemptId',
              'observed_revision': observed?.revision,
              'observed_credential_slot': observed?.credentialSlot,
            },
          })
          as Map,
    ),
  );

  @override
  Future<void> saveCalDavPassword(CalDavAttempt attempt, String password) =>
      _accountWrite(() async {
        final current = await calDavAttempt(attempt.id);
        if (current == null ||
            !const {'prepared', 'waiting'}.contains(current.status) ||
            current.credentialSlot != attempt.credentialSlot) {
          throw const MailOperationFailure(
            'This calendar setup changed before its password was saved.',
          );
        }
        try {
          await credentials.save(attempt.credentialSlot, password, password);
        } catch (_) {
          await call({
            'op': 'wait_calendar_connection',
            'id': attempt.id,
            'error': 'Unlock device credential storage, then retry.',
          });
          throw const MailOperationFailure(
            'Unlock device credential storage, then retry.',
          );
        }
      });

  @override
  Future<void> activateCalDavConnection(
    CalDavAttempt attempt, {
    bool Function()? canDispatch,
  }) async {
    _requireDispatch(canDispatch);
    String? password;
    try {
      password = await credentials.read(attempt.credentialSlot, false);
    } catch (_) {
      await call({
        'op': 'wait_calendar_connection',
        'id': attempt.id,
        'error': 'Unlock device credential storage, then retry.',
      });
      throw const MailOperationFailure(
        'Unlock device credential storage, then retry.',
      );
    }
    if (password == null) {
      await call({
        'op': 'wait_calendar_connection',
        'id': attempt.id,
        'error': 'Unlock device credential storage, then retry.',
      });
      throw const MailOperationFailure(
        'Unlock device credential storage, then retry.',
      );
    }
    final now = DateTime.now().toUtc();
    _requireDispatch(canDispatch);
    await call({
      'op': 'activate_calendar_connection',
      'id': attempt.id,
      'password': password,
      'start': now.subtract(const Duration(days: 365)).toIso8601String(),
      'end': now.add(const Duration(days: 365)).toIso8601String(),
    });
    await _accountWrite(_cleanupCalDavCredentials);
  }

  @override
  Future<List<CalDavAttempt>> calDavAttempts({bool pending = false}) async =>
      ((await call({
                'op': pending
                    ? 'pending_calendar_connection_attempts'
                    : 'calendar_connection_attempts',
              }))
              as List)
          .map(
            (value) => CalDavAttempt(Map<String, dynamic>.from(value as Map)),
          )
          .toList();

  @override
  Future<CalDavAttempt?> calDavAttempt(String id) async {
    final value = await call({'op': 'calendar_connection_attempt', 'id': id});
    return value == null
        ? null
        : CalDavAttempt(Map<String, dynamic>.from(value as Map));
  }

  @override
  Future<List<CalDavConnection>> calDavConnections() async =>
      ((await call({'op': 'active_calendar_connections'})) as List)
          .map(
            (value) =>
                CalDavConnection(Map<String, dynamic>.from(value as Map)),
          )
          .toList();

  @override
  Future<void> cancelCalDavConnection(CalDavAttempt attempt) =>
      _accountWrite(() async {
        await call({'op': 'cancel_calendar_connection', 'id': attempt.id});
        try {
          await _cleanupCalDavCredentials();
        } catch (_) {
          throw const CalendarCredentialCleanupFailure(
            'Unlock device credential storage, then retry cleanup.',
          );
        }
      });

  @override
  Future<void> removeCalDavConnection(CalDavConnection connection) =>
      _accountWrite(() async {
        await call({
          'op': 'remove_calendar_connection',
          'id': connection.id,
          'revision': connection.revision,
        });
        try {
          await _cleanupCalDavCredentials();
        } catch (_) {
          throw const CalendarCredentialCleanupFailure(
            'Unlock device credential storage, then retry cleanup.',
          );
        }
      });

  @override
  Future<void> cleanupCalDavCredentials() =>
      _accountWrite(_cleanupCalDavCredentials);

  Future<void> _cleanupCalDavCredentials() async {
    final slots = (await call({'op': 'calendar_credential_cleanup'}) as List)
        .cast<String>();
    for (final slot in slots) {
      try {
        await credentials.remove(slot);
      } catch (_) {
        throw const MailOperationFailure(
          'Unlock device credential storage, then retry cleanup.',
        );
      }
      await call({'op': 'calendar_credential_cleanup_done', 'slot': slot});
    }
  }

  @override
  Future<CalDavAdmission> admitCalDavAction(
    String actionId,
    CalendarEntry entry,
    CalendarEntry? before,
    CalDavConnection connection,
  ) async => _admitCalDavMutation(actionId, connection, {
    'save': {
      'before': before?.toCalendarJson(),
      'after': entry.toCalendarJson(),
    },
  });

  @override
  Future<CalDavAdmission> admitCalDavDelete(
    String actionId,
    CalendarEntry entry,
    CalDavConnection connection,
  ) async => _admitCalDavMutation(actionId, connection, {
    'delete': {'before': entry.toCalendarJson()},
  });

  Future<CalDavAdmission> _admitCalDavMutation(
    String actionId,
    CalDavConnection connection,
    Map<String, Object?> mutation,
  ) async => CalDavAdmission(
    Map<String, dynamic>.from(
      await call({
            'op': 'admit_cal_dav_action',
            'id': actionId,
            'connection_id': connection.id,
            'connection_revision': connection.revision,
            'mutation': mutation,
          })
          as Map,
    ),
  );

  @override
  Future<CalDavAdmission?> calDavActionAdmission(String actionId) async {
    final value = await call({
      'op': 'cal_dav_action_admission',
      'id': actionId,
    });
    return value == null
        ? null
        : CalDavAdmission(Map<String, dynamic>.from(value as Map));
  }

  @override
  Future<void> executeCalDavAction(
    String actionId,
    String credentialSlot, {
    bool Function()? canDispatch,
  }) async {
    String? password;
    if (canDispatch?.call() == false) return;
    try {
      password = await credentials.read(credentialSlot, false);
    } catch (_) {
      await waitCalendarAction(
        actionId,
        'Unlock device credential storage, then retry.',
      );
      return;
    }
    if (password == null) {
      await waitCalendarAction(
        actionId,
        'Unlock device credential storage, then retry.',
      );
      return;
    }
    if (canDispatch?.call() == false) return;
    await call({
      'op': 'execute_cal_dav_action',
      'id': actionId,
      'password': password,
    });
  }

  @override
  Future<void> inspectCalDavAction(
    String actionId,
    String credentialSlot,
  ) async {
    String? password;
    try {
      password = await credentials.read(credentialSlot, false);
    } catch (_) {
      throw const MailOperationFailure(
        'Unlock device credential storage, then retry.',
      );
    }
    if (password == null) {
      throw const MailOperationFailure(
        'Unlock device credential storage, then retry.',
      );
    }
    await call({
      'op': 'inspect_cal_dav_action',
      'id': actionId,
      'password': password,
    });
  }

  @override
  Future<CalendarAdmission> admitCalendarAction(
    String actionId,
    CalendarEntry entry,
    CalendarEntry? before, {
    required String subject,
  }) async => CalendarAdmission(
    Map<String, dynamic>.from(
      await call({
            'op': 'admit_calendar_action',
            'subject': subject,
            'id': actionId,
            'mutation': {
              'save': {
                'before': before?.toCalendarJson(),
                'after': entry.toCalendarJson(),
              },
            },
          })
          as Map,
    ),
  );

  @override
  Future<CalendarAdmission> admitCalendarDelete(
    String actionId,
    CalendarEntry entry, {
    required String subject,
  }) async => CalendarAdmission(
    Map<String, dynamic>.from(
      await call({
            'op': 'admit_calendar_action',
            'subject': subject,
            'id': actionId,
            'mutation': {
              'delete': {'before': entry.toCalendarJson()},
            },
          })
          as Map,
    ),
  );

  @override
  Future<CalendarAdmission?> calendarActionAdmission(String actionId) async {
    final value = await call({
      'op': 'calendar_action_admission',
      'id': actionId,
    });
    return value == null
        ? null
        : CalendarAdmission(Map<String, dynamic>.from(value as Map));
  }

  @override
  Future<void> executeCalendarAction(
    String actionId,
    String accessToken, {
    required String subject,
  }) async => call({
    'op': 'execute_calendar_action',
    'subject': subject,
    'id': actionId,
    'access_token': accessToken,
  });

  @override
  Future<void> repairCalendarAction(String actionId) async =>
      call({'op': 'repair_calendar_action', 'id': actionId});

  @override
  Future<void> waitCalendarAction(String actionId, String error) async =>
      call({'op': 'wait_calendar_action', 'id': actionId, 'error': error});

  @override
  Future<void> cancelCalendarAction(String actionId) async =>
      call({'op': 'cancel_calendar_action', 'id': actionId});

  @override
  Future<void> acceptCalendarCurrentState(String actionId) async =>
      call({'op': 'accept_calendar_current_state', 'id': actionId});

  @override
  Future<void> inspectCalendarAction(
    String actionId,
    String accessToken, {
    required String subject,
  }) async => call({
    'op': 'inspect_calendar_action',
    'subject': subject,
    'id': actionId,
    'access_token': accessToken,
  });

  @override
  Future<List<CalendarActivity>> calendarActions({int offset = 0}) async =>
      ((await call({'op': 'calendar_actions', 'offset': offset})) as List)
          .map(
            (value) =>
                CalendarActivity(Map<String, dynamic>.from(value as Map)),
          )
          .toList();

  @override
  Future<List<CalendarEntry>> calendarEvents() async {
    final sources = ((await call({'op': 'calendar_sources'})) as List)
        .map(
          (value) =>
              CalendarSource.fromJson(Map<String, dynamic>.from(value as Map)),
        )
        .toList();
    final byId = {for (final source in sources) source.id: source};
    return ((await call({'op': 'calendar_events'})) as List).map((value) {
      final entry = CalendarEntry.fromCalendarJson(
        Map<String, dynamic>.from(value as Map),
      );
      return entry.withSource(
        byId[entry.sourceId] ??
            CalendarSourceView(entry.sourceId, entry.sourceId, true),
      );
    }).toList();
  }

  @override
  Future<CalendarSnapshot> calendarSnapshot() async {
    final value = Map<String, dynamic>.from(
      await call({'op': 'calendar_snapshot'}) as Map,
    );
    final sources = (value['sources'] as List)
        .map(
          (source) =>
              CalendarSource.fromJson(Map<String, dynamic>.from(source as Map)),
        )
        .toList();
    final byId = {for (final source in sources) source.id: source};
    return CalendarSnapshot(
      sources,
      (value['events'] as List).map((raw) {
        final entry = CalendarEntry.fromCalendarJson(
          Map<String, dynamic>.from(raw as Map),
        );
        return entry.withSource(
          byId[entry.sourceId] ??
              CalendarSource(entry.sourceId, entry.sourceId, true),
        );
      }).toList(),
      subject: value['subject'] as String?,
    );
  }

  @override
  Future<CalendarSnapshot> syncCalendar(
    String accessToken,
    DateTime start,
    DateTime end, {
    required String subject,
  }) async {
    final value = Map<String, dynamic>.from(
      await call({
            'op': 'sync_calendar',
            'subject': subject,
            'access_token': accessToken,
            'start': start.toUtc().toIso8601String(),
            'end': end.toUtc().toIso8601String(),
          })
          as Map,
    );
    final sources = (value['sources'] as List)
        .map(
          (source) =>
              CalendarSource.fromJson(Map<String, dynamic>.from(source as Map)),
        )
        .toList();
    final byId = {for (final source in sources) source.id: source};
    return CalendarSnapshot(
      sources,
      (value['events'] as List).map((event) {
        final entry = CalendarEntry.fromCalendarJson(
          Map<String, dynamic>.from(event as Map),
        );
        return switch (byId[entry.sourceId]) {
          final source? => entry.withSource(source),
          null => entry,
        };
      }).toList(),
      subject: value['subject'] as String?,
    );
  }
}
