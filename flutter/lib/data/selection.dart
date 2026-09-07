/// One captured query lives in the repository, independently of loaded rows.
/// Counts and at most 50 observed identities cross each repository call.
abstract interface class SelectionRepository {
  Future<dynamic> selection(
    Map<String, Object?> command, {
    List<String> observed = const [],
  });
}

class SelectionSnapshot {
  SelectionSnapshot(Map<String, dynamic> data)
    : id = data['id'],
      revision = data['revision'],
      total = data['total'],
      selected = data['selected'],
      available = data['available'],
      unread = data['unread'],
      starred = data['starred'],
      frozen = data['frozen'],
      visible = (data['visible'] as List).cast<String>().toSet(),
      positions = (data['positions'] as Map).cast<String, int>(),
      aliases = (data['aliases'] as Map? ?? {}).cast<String, String>(),
      groups = (data['groups'] as List).cast<Map<String, dynamic>>();
  final String id;
  final int revision, total, selected, available, unread, starred;
  final bool frozen;
  final Set<String> visible;
  final Map<String, int> positions;
  final Map<String, String> aliases;
  final List<Map<String, dynamic>> groups;
}
