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
  String get status => data['status'] as String;
  String? get error => data['error'] as String?;
  Map<String, Object> get fields {
    final fields = Map<String, Object>.from(data['fields'] as Map)
      ..removeWhere((_, value) => value == null);
    if (fields['folder'] == 'INBOX') fields['folder'] = 'Inbox';
    return fields;
  }

  bool get needsReview =>
      const {'rejected', 'uncertain', 'repair'}.contains(status);
  bool get canResume => const {'queued', 'waiting'}.contains(status);
}

abstract interface class MailActivityRepository {
  Future<List<MailActivity>> mailActions();
  Future<void> resumeMailAction(MailActivity action);
  Future<void> cancelMailAction(String id);
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
