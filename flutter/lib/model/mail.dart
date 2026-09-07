import 'dart:math';

class ReceivedAttachment {
  const ReceivedAttachment({
    required this.id,
    required this.name,
    required this.mediaType,
    required this.size,
    this.contentId,
  });
  final String? contentId;
  final String id, name, mediaType;
  final int size;
  factory ReceivedAttachment.fromJson(Map<String, dynamic> value) =>
      ReceivedAttachment(
        id: value['id'],
        name: value['name'],
        mediaType: value['media_type'],
        size: value['size'],
        contentId: value['content_id'],
      );
}

enum MailAction {
  none('None'),
  archive('Archive'),
  trash('Trash'),
  read('Read / unread'),
  star('Flag / unflag'),
  select('Select'),
  move('Move'),
  spam('Spam');

  const MailAction(this.label);
  final String label;
}

class Mail {
  const Mail({
    required this.id,
    required this.sender,
    required this.address,
    required this.subject,
    required this.preview,
    required this.body,
    required this.date,
    this.account = 'Personal',
    this.folder = 'Inbox',
    this.unread = true,
    this.starred = false,
    this.attachments = const [],
    this.files = const [],
    this.fileError,
    this.accountId = '',
    this.bodyLoaded = true,
  });
  final String id, sender, address, subject, preview, body, account, folder;
  final DateTime date;
  final bool unread, starred;
  final List<String> attachments;
  final List<ReceivedAttachment> files;
  final String? fileError;
  final String accountId;
  final bool bodyLoaded;

  Mail patch(Map<String, Object> fields) => Mail(
    id: fields['id'] as String? ?? id,
    sender: sender,
    address: address,
    subject: subject,
    preview: preview,
    body: body,
    date: date,
    account: account,
    folder: fields['folder'] as String? ?? folder,
    unread: fields['unread'] as bool? ?? unread,
    starred: fields['starred'] as bool? ?? starred,
    attachments: attachments,
    files: files,
    fileError: fileError,
    accountId: accountId,
    bodyLoaded: bodyLoaded,
  );

  Mail withoutBody() => Mail(
    id: id,
    sender: sender,
    address: address,
    subject: subject,
    preview: preview,
    body: '',
    date: date,
    account: account,
    accountId: accountId,
    folder: folder,
    unread: unread,
    starred: starred,
    attachments: attachments,
    files: files,
    fileError: fileError,
    bodyLoaded: false,
  );

  Mail withDetail(Mail detail) => Mail(
    id: id,
    sender: sender,
    address: address,
    subject: subject,
    preview: preview,
    body: detail.body,
    date: date,
    account: account,
    accountId: accountId,
    folder: folder,
    unread: unread,
    starred: starred,
    attachments: detail.attachments,
    files: detail.files,
    fileError: detail.fileError,
    bodyLoaded: true,
  );

  Object field(String name) => switch (name) {
    'folder' => folder,
    'unread' => unread,
    'starred' => starred,
    _ => throw ArgumentError.value(name),
  };
}

class CalendarEntry {
  const CalendarEntry(
    this.id,
    this.title,
    this.start,
    this.end, {
    this.calendar = 'Personal',
    this.location = '',
    this.readOnly = false,
  });
  final String id, title, calendar, location;
  final DateTime start, end;
  final bool readOnly;
}

class DraftAttachment {
  const DraftAttachment({
    required this.id,
    required this.name,
    required this.mediaType,
    required this.size,
    this.contentId,
  });
  final String? contentId;
  final String id, name, mediaType;
  final int size;
  Map<String, Object?> toJson() => {
    'content_id': contentId,
    'id': id,
    'name': name,
    'media_type': mediaType,
    'size': size,
  };
  factory DraftAttachment.fromJson(Map<String, dynamic> value) =>
      DraftAttachment(
        id: value['id'],
        name: value['name'],
        mediaType: value['media_type'],
        size: value['size'],
        contentId: value['content_id'],
      );
}

String newDraftIdentity() {
  final random = Random.secure();
  final bytes = List.generate(16, (_) => random.nextInt(256));
  bytes[6] = (bytes[6] & 15) | 64;
  bytes[8] = (bytes[8] & 63) | 128;
  final hex = bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
  return '${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-${hex.substring(16, 20)}-${hex.substring(20)}';
}

class ForwardQuote {
  const ForwardQuote({
    required this.text,
    required this.htmlHead,
    required this.htmlAttributes,
    required this.htmlBody,
  });
  final String text, htmlHead, htmlAttributes, htmlBody;
  factory ForwardQuote.fromJson(Map<String, dynamic> value) => ForwardQuote(
    text: value['text'],
    htmlHead: value['html_head'],
    htmlAttributes: value['html_attributes'] ?? '',
    htmlBody: value['html_body'],
  );
  Map<String, String> toJson() => {
    'text': text,
    'html_head': htmlHead,
    'html_attributes': htmlAttributes,
    'html_body': htmlBody,
  };
}

class Draft {
  const Draft({
    required this.id,
    this.to = '',
    this.cc = '',
    this.bcc = '',
    this.subject = '',
    this.body = '',
    this.accountId = '',
    this.revision = 0,
    this.fileRevision = 0,
    this.forward,
    this.inReplyTo,
    this.references = const [],
    this.attachments = const [],
  });
  final String id, to, cc, bcc, subject, body;
  final String accountId;
  final int revision, fileRevision;
  final String? inReplyTo;
  final ForwardQuote? forward;
  final List<String> references;
  final List<DraftAttachment> attachments;
  Map<String, Object?> toJson() => {
    'id': id,
    'account_id': accountId,
    'to': to,
    'cc': cc,
    'bcc': bcc,
    'subject': subject,
    'body': body,
    'revision': revision,
    'in_reply_to': inReplyTo,
    'references': references,
    'file_revision': fileRevision,
    'forward': forward?.toJson(),
    'attachments': attachments.map((a) => a.toJson()).toList(),
  };
  factory Draft.fromJson(Map<String, dynamic> json) => Draft(
    id: json['id'],
    accountId: json['account_id'],
    to: json['to'],
    cc: json['cc'],
    bcc: json['bcc'],
    subject: json['subject'],
    body: json['body'],
    revision: json['revision'],
    fileRevision: json['file_revision'] ?? 0,
    forward: json['forward'] == null
        ? null
        : ForwardQuote.fromJson(json['forward']),
    inReplyTo: json['in_reply_to'],
    references: (json['references'] as List? ?? []).cast<String>(),
    attachments: (json['attachments'] as List? ?? [])
        .map((a) => DraftAttachment.fromJson(a))
        .toList(),
  );
}
