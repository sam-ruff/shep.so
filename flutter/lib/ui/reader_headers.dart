import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../model/mail.dart';
import 'format.dart';
import 'icons.dart';
import 'theme.dart';

class ReaderHeaders extends StatefulWidget {
  const ReaderHeaders({super.key, required this.mail});
  final Mail mail;
  @override
  State<ReaderHeaders> createState() => _ReaderHeadersState();
}

class _ReaderHeadersState extends State<ReaderHeaders> {
  int revision = 0;
  String? feedback;
  bool failed = false;
  Mail get mail => widget.mail;

  @override
  void didUpdateWidget(ReaderHeaders oldWidget) {
    super.didUpdateWidget(oldWidget);
    final old = oldWidget.mail;
    if (old.id == mail.id &&
        old.subject == mail.subject &&
        old.sender == mail.sender &&
        old.senderHeader == mail.senderHeader &&
        old.address == mail.address &&
        old.recipient == mail.recipient) {
      return;
    }
    revision++;
    feedback = null;
  }

  Future<void> copy(String label, String value) async {
    final attempt = ++revision;
    try {
      await Clipboard.setData(ClipboardData(text: value));
      if (!mounted || attempt != revision) return;
      setState(() {
        failed = false;
        feedback = '$label copied.';
      });
    } on Object {
      if (!mounted || attempt != revision) return;
      setState(() {
        failed = true;
        feedback = 'Could not copy $label. Try Copy again.';
      });
    }
  }

  Widget value(
    BuildContext context,
    String key,
    String label,
    String text,
    TextStyle style, {
    bool heading = false,
  }) {
    final empty = text.trim().isEmpty;
    final title = '${label[0].toUpperCase()}${label.substring(1)}';
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (!heading)
                Text(
                  title,
                  style: TextStyle(
                    color: ShepColors.of(context).muted,
                    fontSize: ShepText.caption,
                  ),
                ),
              if (empty)
                Text(
                  '$title not provided',
                  key: ValueKey('reader-$key-empty'),
                  style: style,
                )
              else
                SelectableText(
                  text,
                  key: ValueKey('reader-$key'),
                  style: style,
                ),
            ],
          ),
        ),
        const SizedBox(width: 8),
        IconButton(
          key: ValueKey('reader-copy-$key'),
          tooltip: 'Copy $label',
          onPressed: empty ? null : () => copy(label, text),
          icon: const ShepIcon('copy', size: 18),
          constraints: const BoxConstraints(minWidth: 44, minHeight: 44),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    final secondary = TextStyle(color: c.muted, fontSize: ShepText.caption);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        value(
          context,
          'subject',
          'subject',
          mail.subject,
          TextStyle(
            fontSize: ShepText.heading,
            fontWeight: FontWeight.w600,
            color: c.text,
          ),
          heading: true,
        ),
        const SizedBox(height: 16),
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              width: 41,
              height: 41,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: avatarColors(0).$1,
                shape: BoxShape.circle,
              ),
              child: Text(
                avatarInitials(mail.sender),
                style: TextStyle(
                  fontSize: 15,
                  fontWeight: FontWeight.w600,
                  color: avatarColors(0).$2,
                ),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: value(
                context,
                'sender',
                'sender',
                mail.senderHeader ?? mail.sender,
                TextStyle(
                  fontSize: ShepText.body,
                  fontWeight: FontWeight.w600,
                  color: c.text,
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: 8),
        value(context, 'address', 'sender address', mail.address, secondary),
        const SizedBox(height: 8),
        value(context, 'recipient', 'To', mail.recipient, secondary),
        const SizedBox(height: 8),
        Text(
          'Account: ${mail.account}',
          key: const ValueKey('reader-account'),
          style: secondary,
        ),
        const SizedBox(height: 6),
        Text(readerDate(mail.date), style: secondary),
        if (feedback != null)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: Semantics(
              liveRegion: true,
              child: Text(
                feedback!,
                key: const ValueKey('reader-header-feedback'),
                style: TextStyle(
                  color: failed ? Theme.of(context).colorScheme.error : c.muted,
                ),
              ),
            ),
          ),
      ],
    );
  }
}
