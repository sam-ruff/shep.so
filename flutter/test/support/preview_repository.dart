import 'selection_repository.dart';
import 'package:shep_mobile/data/selection.dart';
import 'dart:convert';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'fixture_json.dart';

class PreviewRepository implements MailRepository, SelectionRepository {
  PreviewRepository({
    this.delay = const Duration(milliseconds: 350),
    this.fail = false,
    String? firstBody,
  }) {
    final data = jsonDecode(fixtureJson) as Map<String, dynamic>;
    if (firstBody != null) data['messages'][0]['body'] = firstBody;
    _mail = (data['messages'] as List).map((raw) {
      final m = raw as Map<String, dynamic>;
      return Mail(
        id: m['id'],
        sender: m['sender'],
        address: m['address'],
        subject: m['subject'],
        preview: m['preview'],
        body: m['body'],
        account: m['account'],
        folder: m['folder'],
        date: DateTime.parse(m['date']),
        unread: m['unread'],
        starred: m['starred'],
        attachments: List<String>.from(m['attachments']),
      );
    }).toList();
    _events = (data['events'] as List)
        .map(
          (e) => CalendarEntry(
            e['id'],
            e['title'],
            DateTime.parse(e['start']),
            DateTime.parse(e['end']),
            calendar: e['calendar'],
            location: e['location'],
            readOnly: e['readOnly'],
          ),
        )
        .toList();
  }
  late final selectionPreview = PreviewSelectionRepository(() => cached);
  @override
  Future<dynamic> selection(
    Map<String, Object?> command, {
    List<String> observed = const [],
  }) => selectionPreview.selection(command, observed: observed);
  final Duration delay;
  bool fail;
  late List<Mail> _mail;
  late List<CalendarEntry> _events;
  final Map<String, Draft> drafts = {};
  @override
  bool get preview => true;
  @override
  List<Mail> get cached => List.unmodifiable(_mail);
  @override
  List<CalendarEntry> get events => List.unmodifiable(_events);
  @override
  Future<List<Mail>> refresh() async {
    await Future<void>.delayed(delay);
    if (fail) throw StateError('Fixture rejection');
    return cached;
  }

  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    await Future<void>.delayed(delay);
    if (fail) throw StateError('Fixture rejection');
    _mail = _mail.map((m) => m.id == id ? m.patch(fields) : m).toList();
  }

  @override
  Future<void> saveDraft(Draft draft) async {
    await Future<void>.delayed(delay);
    if (fail) throw StateError('Fixture rejection');
    drafts[draft.id] = draft;
  }

  @override
  Future<void> send(Draft draft) async =>
      throw StateError('Preview cannot send');
  @override
  Future<void> saveEvent(CalendarEntry event) async {
    await Future<void>.delayed(delay);
    if (fail || _events.any((e) => e.id == event.id && e.readOnly)) {
      throw StateError('Fixture rejection');
    }
    _events = [..._events.where((e) => e.id != event.id), event];
  }
}
