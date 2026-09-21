import '../model/mail.dart';

/// Provider changes must acknowledge only committed fields. Never replace a whole
/// message with an old body/result after a later flag or move.
abstract interface class MailRepository {
  bool get preview;
  List<Mail> get cached;
  List<CalendarEntry> get events;
  Future<List<Mail>> refresh();
  Future<void> mutate(String id, Map<String, Object> fields);
  Future<void> saveDraft(Draft draft);
  Future<void> send(Draft draft);
  Future<void> saveEvent(CalendarEntry event);
}

class MailActivity {
  const MailActivity(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'] as String;
  String get mail => data['mail'] as String;
  String get account => data['account'] as String;
  int get created => data['created'] as int? ?? 0;
  String get status => data['status'] as String;
  String? get error => data['error'] as String?;
  Map<String, Object> get fields {
    final fields = Map<String, dynamic>.from(data['fields'] as Map)
      ..removeWhere((_, value) => value == null);
    if (fields['folder'] == 'INBOX') fields['folder'] = 'Inbox';
    return Map<String, Object>.from(fields);
  }

  bool get needsReview =>
      const {'rejected', 'uncertain', 'repair'}.contains(status);
  bool get canResume => const {'queued', 'waiting'}.contains(status);
  bool get canUndo => status == 'succeeded';
}

abstract interface class MailActivityRepository {
  Future<List<MailActivity>> mailActions({int offset = 0});
  Future<List<MailActivity>> runnableMailActions({
    int? afterCreated,
    String? afterId,
  });
  Future<void> resumeMailAction(MailActivity action);
  Future<void> cancelMailAction(String id);
  Future<void> undoMailAction(
    MailActivity action, {
    void Function()? onAdmitted,
  });
  Future<void> inspectMailAction(MailActivity action);
}

abstract interface class DurableMutationRepository {
  Future<void> admitMutation(
    String id,
    Map<String, Object> fields,
    String actionId,
    String observedLineage,
  );
  Future<void> executeMutation(
    String id,
    Map<String, Object> fields,
    String actionId,
  );
  Future<void> cancelAdmittedMutation(String actionId);
}

class CalendarActivity {
  const CalendarActivity(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'] as String;
  String get status => data['status'] as String;
  String? get error => data['error'] as String?;
  String? get subject => data['subject'] as String?;
  String? get connectionId => data['connection_id'] as String?;
  int? get connectionRevision => data['connection_revision'] as int?;
  String? get credentialSlot => data['credential_slot'] as String?;
  bool get isCalDav => connectionId != null;
  bool get canResume => const {'queued', 'waiting', 'repair'}.contains(status);
  bool get canCancel =>
      const {'queued', 'waiting', 'rejected'}.contains(status);
  bool get canInspect => status == 'uncertain';
  bool get canAcceptCurrent => status == 'uncertain' && checked;
  bool get checked => data['checked'] as bool? ?? false;
  Map<String, dynamic> get _mutation =>
      Map<String, dynamic>.from(data['mutation'] as Map);
  bool get isDelete => _mutation.containsKey('delete');
  String get statusLabel => switch (status) {
    'queued' => isDelete ? 'Deletion queued' : 'Event queued',
    'running' => isDelete ? 'Deleting event' : 'Saving event',
    'waiting' => isDelete ? 'Deletion waiting' : 'Event waiting',
    'uncertain' =>
      isDelete ? 'Deletion needs checking' : 'Event needs checking',
    'repair' => 'Saving on this device',
    'rejected' => isDelete ? 'Event was not deleted' : 'Event was not saved',
    _ => 'Calendar activity',
  };
  Map<String, dynamic>? get _save => _mutation['save'] == null
      ? null
      : Map<String, dynamic>.from(_mutation['save'] as Map);
  Map<String, dynamic> get _delete =>
      Map<String, dynamic>.from(_mutation['delete'] as Map);
  CalendarEntry? get before => isDelete
      ? CalendarEntry.fromCalendarJson(
          Map<String, dynamic>.from(_delete['before'] as Map),
        )
      : _save!['before'] == null
      ? null
      : CalendarEntry.fromCalendarJson(
          Map<String, dynamic>.from(_save!['before'] as Map),
        );
  CalendarEntry get requested => isDelete
      ? before!
      : CalendarEntry.fromCalendarJson(
          Map<String, dynamic>.from(_save!['after'] as Map),
        );
  CalendarEntry? get saved {
    final receipt = data['receipt'];
    if (receipt is! Map || receipt['after'] == null) return null;
    return CalendarEntry.fromCalendarJson(
      Map<String, dynamic>.from(receipt['after'] as Map),
    );
  }
}

class CalendarAdmission {
  const CalendarAdmission(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'] as String;
  String get status => data['status'] as String;
  String get subject => data['subject'] as String;
  Map<String, dynamic> get mutation =>
      Map<String, dynamic>.from(data['mutation'] as Map);
  CalendarEntry? get saved {
    final receipt = data['receipt'];
    if (receipt is! Map || receipt['after'] == null) return null;
    return CalendarEntry.fromCalendarJson(
      Map<String, dynamic>.from(receipt['after'] as Map),
    );
  }
}

class CalendarSource extends CalendarSourceView {
  const CalendarSource(super.id, super.name, super.readOnly);
  factory CalendarSource.fromJson(Map<String, dynamic> json) => CalendarSource(
    json['id'] as String,
    json['name'] as String,
    json['read_only'] as bool? ?? false,
  );
}

class CalendarSnapshot {
  const CalendarSnapshot(this.sources, this.events, {this.subject});
  final List<CalendarSource> sources;
  final List<CalendarEntry> events;
  final String? subject;
}

class CalDavConnection {
  const CalDavConnection(this.data);
  final Map<String, dynamic> data;
  Map<String, dynamic> get connection =>
      Map<String, dynamic>.from(data['connection'] as Map);
  String get id => connection['id'] as String;
  String get url => connection['url'] as String;
  String get username => connection['username'] as String;
  String get credentialSlot => data['credential_slot'] as String;
  int get revision => data['revision'] as int;
}

class CalDavAttempt {
  const CalDavAttempt(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'] as String;
  String get status => data['status'] as String;
  String? get error => data['error'] as String?;
  Map<String, dynamic> get request =>
      Map<String, dynamic>.from(data['request'] as Map);
  String get connectionId =>
      Map<String, dynamic>.from(request['connection'] as Map)['id'] as String;
  String get url =>
      Map<String, dynamic>.from(request['connection'] as Map)['url'] as String;
  String get username =>
      Map<String, dynamic>.from(request['connection'] as Map)['username']
          as String;
  String get credentialSlot => request['credential_slot'] as String;
}

class CalDavAdmission {
  const CalDavAdmission(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'] as String;
  String get status => data['status'] as String;
  String get connectionId => data['connection_id'] as String;
  int get connectionRevision => data['connection_revision'] as int;
  String get credentialSlot => data['credential_slot'] as String;
  CalendarEntry? get saved {
    final receipt = data['receipt'];
    if (receipt is! Map || receipt['after'] == null) return null;
    return CalendarEntry.fromCalendarJson(
      Map<String, dynamic>.from(receipt['after'] as Map),
    );
  }
}

abstract interface class DurableCalDavRepository {
  Future<CalDavAttempt> admitCalDavConnection({
    required String attemptId,
    required String connectionId,
    required String url,
    required String username,
    CalDavConnection? observed,
  });
  Future<void> saveCalDavPassword(CalDavAttempt attempt, String password);
  Future<void> activateCalDavConnection(CalDavAttempt attempt);
  Future<List<CalDavAttempt>> calDavAttempts({bool pending = false});
  Future<CalDavAttempt?> calDavAttempt(String id);
  Future<List<CalDavConnection>> calDavConnections();
  Future<void> cancelCalDavConnection(CalDavAttempt attempt);
  Future<void> removeCalDavConnection(CalDavConnection connection);
  Future<void> cleanupCalDavCredentials();
  Future<CalDavAdmission> admitCalDavAction(
    String actionId,
    CalendarEntry entry,
    CalendarEntry? before,
    CalDavConnection connection,
  );
  Future<CalDavAdmission> admitCalDavDelete(
    String actionId,
    CalendarEntry entry,
    CalDavConnection connection,
  );
  Future<CalDavAdmission?> calDavActionAdmission(String actionId);
  Future<void> executeCalDavAction(String actionId, String credentialSlot);
  Future<void> inspectCalDavAction(String actionId, String credentialSlot);
}

class CalendarCredentialCleanupFailure implements Exception {
  const CalendarCredentialCleanupFailure(this.message);
  final String message;
  @override
  String toString() => message;
}

abstract interface class DurableCalendarRepository {
  Future<CalendarAdmission> admitCalendarAction(
    String actionId,
    CalendarEntry entry,
    CalendarEntry? before, {
    required String subject,
  });
  Future<CalendarAdmission> admitCalendarDelete(
    String actionId,
    CalendarEntry entry, {
    required String subject,
  });
  Future<void> executeCalendarAction(
    String actionId,
    String accessToken, {
    required String subject,
  });
  Future<void> repairCalendarAction(String actionId);
  Future<void> waitCalendarAction(String actionId, String error);
  Future<void> cancelCalendarAction(String actionId);
  Future<void> acceptCalendarCurrentState(String actionId);
  Future<CalendarAdmission?> calendarActionAdmission(String actionId);
  Future<void> inspectCalendarAction(
    String actionId,
    String accessToken, {
    required String subject,
  });
  Future<List<CalendarActivity>> calendarActions({int offset = 0});
  Future<List<CalendarEntry>> calendarEvents();
  Future<CalendarSnapshot> calendarSnapshot();
  Future<CalendarSnapshot> syncCalendar(
    String accessToken,
    DateTime start,
    DateTime end, {
    required String subject,
  });
}

/// Production startup is empty until the native provider adapter is connected.
/// It cannot turn a button click into a false server acknowledgment.
class UnconnectedRepository implements MailRepository {
  @override
  bool get preview => false;
  @override
  List<Mail> get cached => const [];
  @override
  List<CalendarEntry> get events => const [];
  Never _unavailable() => throw StateError(
    'Provider connection is not available in this development build. Use the Rust desktop client for live mail.',
  );
  @override
  Future<List<Mail>> refresh() async => _unavailable();
  @override
  Future<void> mutate(String id, Map<String, Object> fields) async =>
      _unavailable();
  @override
  Future<void> saveDraft(Draft draft) async => _unavailable();
  @override
  Future<void> send(Draft draft) async => _unavailable();
  @override
  Future<void> saveEvent(CalendarEntry event) async => _unavailable();
}
