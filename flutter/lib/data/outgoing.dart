class OutgoingEntry {
  const OutgoingEntry({
    required this.id,
    required this.draftId,
    required this.accountId,
    required this.subject,
    required this.to,
    required this.from,
    required this.state,
    this.sent,
    this.sentError,
    this.protocol,
    this.sentPolicy,
    this.marked = false,
  });
  final String id, draftId, accountId, subject, to, from, state;
  final String? sent, sentError, protocol, sentPolicy;
  final bool marked;
  bool get copyUncertain => sent == 'uncertain' || sent == 'appending';
  bool get canCheckSent => protocol == 'Imap' && (uncertain || delivered);
  bool get canCopySent =>
      canCheckSent && delivered && sentPolicy == 'Automatic';
  bool get uncertain => state == 'uncertain' && !marked && sent != 'saved';
  bool get active => state == 'submitting';
  bool get delivered => state == 'delivered' || marked || sent == 'saved';
  String get label => sent == 'saved' && state == 'uncertain'
      ? 'Matching copy confirmed in Sent'
      : marked
      ? 'Recorded as sent after review'
      : switch (state) {
          'submitting' => 'Delivery in progress',
          'rejected' => 'Not sent',
          'delivered' => 'Delivery confirmed',
          _ => 'Delivery not confirmed',
        };
  factory OutgoingEntry.fromJson(Map<String, dynamic> value) => OutgoingEntry(
    id: value['id'],
    draftId: value['draft_id'],
    accountId: value['account_id'],
    subject: value['subject'],
    to: value['to'],
    from: value['from'],
    state: value['state'],
    sent: value['sent'],
    sentError: value['sent_error'],
    protocol: value['protocol'],
    sentPolicy: value['sent_policy'],
    marked: value['marked'] ?? false,
  );
}

class OutgoingPage {
  const OutgoingPage(this.rows, this.offset, this.total);
  final List<OutgoingEntry> rows;
  final int offset, total;
  factory OutgoingPage.fromJson(Map<String, dynamic> value) => OutgoingPage(
    (value['rows'] as List).map((r) => OutgoingEntry.fromJson(r)).toList(),
    value['offset'],
    value['total'],
  );
}

enum OutgoingAction { check, checkSent, copySent, backToDrafts, mark, local }

class OutgoingResult {
  const OutgoingResult({
    this.state,
    this.recovery,
    this.draftId,
    this.sent,
    this.notice,
  });
  final String? state, recovery, draftId, sent, notice;
  factory OutgoingResult.fromJson(Map<String, dynamic> value) => OutgoingResult(
    state: value['state'],
    recovery: value['recovery'],
    draftId: value['draft_id'],
    sent: value['sent'],
    notice: value['notice'],
  );
}

abstract interface class OutgoingRepository {
  Future<OutgoingPage> outbox({int offset = 0});
  Future<OutgoingResult> recoverOutgoing(
    String id,
    OutgoingAction action, {
    bool confirmed = false,
  });
}
