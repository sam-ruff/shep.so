/// Durable group actions live in the repository's journal. Dart sees one
/// review, at most 20 History entries and 50-item pages, never a whole group.
abstract interface class GroupRepository {
  Future<dynamic> groups(Map<String, Object?> command);

  /// Runs at most one owned step, supplying the account credential when the
  /// journal asks for it. Returns the journal's step reply.
  Future<Map<String, dynamic>> groupStep();
}

enum GroupAction {
  archive('Archive', 'archive'),
  delete('Delete', 'trash'),
  move('Move', 'move'),
  read('Mark read', 'mail-open'),
  unread('Mark unread', 'mail'),
  flag('Flag', 'flag'),
  unflag('Unflag', 'flag');

  const GroupAction(this.label, this.icon);
  final String label, icon;

  Map<String, Object?> toJson({String? folder}) => switch (this) {
    GroupAction.move => {'kind': 'move', 'folder': folder},
    _ => {'kind': name},
  };
}

/// Terminal item states that count as progress for a running group.
const groupSettledStates = {
  'done',
  'skipped',
  'failed',
  'uncertain',
  'cancelled',
  'accepted',
  'undone',
  'undo_skipped',
  'undo_failed',
  'undo_uncertain',
};
const groupAttentionStates = {
  'failed',
  'uncertain',
  'undo_failed',
  'undo_uncertain',
};

class GroupJob {
  GroupJob(Map<String, dynamic> data)
    : id = data['id'],
      state = data['state'],
      undo = data['undo'] == true,
      total = data['total'] ?? 0,
      revision = data['revision'] ?? 0,
      created = data['created'] ?? 0,
      error = data['error'],
      action = Map<String, dynamic>.from(data['action'] as Map? ?? {}),
      counts = (data['counts'] as Map? ?? {}).map(
        (k, v) => MapEntry(k as String, v as int),
      ),
      groups = (data['groups'] as List? ?? []).cast<Map<String, dynamic>>();
  final String id, state;
  final bool undo;
  final int total, revision, created;
  final String? error;
  final Map<String, dynamic> action;
  final Map<String, int> counts;
  final List<Map<String, dynamic>> groups;

  int count(String state) => counts[state] ?? 0;
  int get settled =>
      groupSettledStates.fold(0, (sum, state) => sum + count(state));
  int get attention =>
      groupAttentionStates.fold(0, (sum, state) => sum + count(state));
  bool get active =>
      state == 'running' || state == 'undoing' || state == 'paused';
  bool get finished => state == 'finished';
  bool get paused => state == 'paused';
  bool get inReview => state == 'review' || state == 'staging';
  bool get canUndo => !undo && (active || finished) && count('done') > 0;
  bool get canPause => state == 'running' || state == 'undoing';
  bool get canRemove =>
      state == 'finished' || state == 'cancelled' || state == 'interrupted';

  String get kind => action['kind'] as String? ?? 'archive';
  String? get folder => action['folder'] as String?;

  /// Human label of the requested change, e.g. "Archive" or "Move to Work".
  String get verb => switch (kind) {
    'archive' => 'Archive',
    'delete' => 'Delete',
    'move' => 'Move to ${folder == 'INBOX' ? 'Inbox' : folder}',
    'read' => 'Mark read',
    'unread' => 'Mark unread',
    'flag' => 'Flag',
    'unflag' => 'Unflag',
    _ => kind,
  };
  String get past => switch (kind) {
    'archive' => 'Archived',
    'delete' => 'Deleted',
    'move' => 'Moved',
    'read' => 'Marked read',
    'unread' => 'Marked unread',
    'flag' => 'Flagged',
    'unflag' => 'Unflagged',
    _ => kind,
  };
  String get progressive => switch (kind) {
    'archive' => 'Archiving',
    'delete' => 'Deleting',
    'move' => 'Moving',
    'read' => 'Marking read',
    'unread' => 'Marking unread',
    'flag' => 'Flagging',
    'unflag' => 'Unflagging',
    _ => kind,
  };
  String get noun => total == 1 ? 'message' : 'messages';

  String get title => '$verb $total $noun';
  String get status {
    if (inReview) return 'Waiting for approval';
    if (state == 'undoing') return 'Undoing, ${count('undone')} restored';
    if (state == 'paused') {
      return attention > 0
          ? 'Paused, $attention need${attention == 1 ? 's' : ''} review'
          : 'Paused';
    }
    if (state == 'running') return '$progressive, $settled of $total';
    if (undo) {
      final restored = count('undone');
      return 'Undone: $restored restored'
          '${attention > 0 ? ', $attention need review' : ''}';
    }
    final done = count('done');
    final skipped = count('skipped');
    return '$past $done'
        '${skipped > 0 ? ', $skipped skipped' : ''}'
        '${attention > 0 ? ', $attention need review' : ''}';
  }
}

class GroupItem {
  GroupItem(Map<String, dynamic> data)
    : position = data['position'],
      mail = data['mail'],
      state = data['state'],
      reason = data['reason'],
      subject = data['subject'] ?? 'Message no longer cached',
      sender = data['sender'] ?? '',
      folder = data['folder'] ?? '',
      account = data['account'] ?? '';
  final int position;
  final String mail, state, subject, sender, folder, account;
  final String? reason;
  bool get needsAttention => groupAttentionStates.contains(state);
  bool get canRetry => state == 'failed' || state == 'undo_failed';
  bool get canAccept => state == 'uncertain' || state == 'undo_uncertain';
  String get label => switch (state) {
    'pending' => 'Waiting',
    'sending' => 'Sending',
    'done' => 'Done',
    'skipped' => 'Skipped',
    'failed' => 'Failed',
    'uncertain' => 'Unconfirmed',
    'cancelled' => 'Cancelled',
    'accepted' => 'Accepted',
    'undoing' => 'Waiting for Undo',
    'reversing' => 'Undoing',
    'undone' => 'Restored',
    'undo_skipped' => 'Undo skipped',
    'undo_failed' => 'Undo failed',
    'undo_uncertain' => 'Undo unconfirmed',
    _ => state,
  };
}
