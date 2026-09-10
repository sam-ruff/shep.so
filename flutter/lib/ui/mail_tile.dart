import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'theme.dart';

class MailTile extends StatelessWidget {
  const MailTile({
    super.key,
    required this.mail,
    required this.workspace,
    required this.open,
    required this.act,
  });
  final Mail mail;
  final Workspace workspace;
  final VoidCallback open;
  final void Function(MailAction) act;

  String label(MailAction action) => switch (action) {
    MailAction.read => mail.unread ? 'Mark read' : 'Mark unread',
    MailAction.star => mail.starred ? 'Unflag' : 'Flag',
    MailAction.select =>
      workspace.selected.contains(mail.id) ? 'Deselect' : 'Select',
    _ => action.label,
  };

  IconData icon(MailAction action) => switch (action) {
    MailAction.read =>
      mail.unread
          ? Icons.mark_email_read_outlined
          : Icons.mark_email_unread_outlined,
    MailAction.star => mail.starred ? Icons.flag : Icons.flag_outlined,
    _ => actionIcon(action.name),
  };

  Widget background(BuildContext context, MailAction action, bool left) {
    final scheme = Theme.of(context).colorScheme;
    final destructive = action == MailAction.trash || action == MailAction.spam;
    final color = destructive ? scheme.error : scheme.primary;
    final foreground = destructive ? scheme.onError : scheme.onPrimary;
    return Container(
      color: color,
      padding: const EdgeInsets.symmetric(horizontal: 24),
      alignment: left ? Alignment.centerRight : Alignment.centerLeft,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon(action), color: foreground, size: 25),
          const SizedBox(height: 5),
          Text(
            label(action),
            style: TextStyle(color: foreground, fontSize: 12),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final prefs = workspace.preferences;
    final selected = workspace.selected.contains(mail.id);
    final row = Material(
      color: selected
          ? scheme.primaryContainer.withValues(alpha: .4)
          : mail.unread
          ? scheme.surface
          : scheme.surfaceContainerLow.withValues(alpha: .45),
      child: InkWell(
        onTap: workspace.selected.isNotEmpty
            ? () => act(MailAction.select)
            : open,
        onLongPress: () => act(MailAction.select),
        child: Container(
          decoration: BoxDecoration(
            border: Border(
              left: BorderSide(
                color: mail.account == 'Personal'
                    ? scheme.primary
                    : const Color(0xff49958a),
                width: 3,
              ),
              bottom: BorderSide(color: scheme.outlineVariant),
            ),
          ),
          padding: const EdgeInsets.fromLTRB(13, 13, 4, 13),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (prefs.avatars || selected)
                Padding(
                  padding: const EdgeInsets.only(right: 12, top: 2),
                  child: CircleAvatar(
                    radius: 20,
                    backgroundColor: scheme.surfaceContainerHighest,
                    child: selected
                        ? Icon(Icons.check, color: scheme.primary)
                        : Text(
                            mail.sender.substring(0, 1),
                            style: TextStyle(
                              color: scheme.onSurfaceVariant,
                              fontSize: 15,
                            ),
                          ),
                  ),
                ),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            mail.sender,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontWeight: mail.unread
                                  ? FontWeight.w600
                                  : FontWeight.w400,
                              fontSize: 14,
                            ),
                          ),
                        ),
                        Text(
                          '${mail.date.hour.toString().padLeft(2, '0')}:${mail.date.minute.toString().padLeft(2, '0')}',
                          style: TextStyle(
                            color: mail.unread
                                ? scheme.primary
                                : scheme.onSurfaceVariant,
                            fontSize: 11,
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 3),
                    Text(
                      mail.subject,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 13,
                        fontWeight: mail.unread
                            ? FontWeight.w600
                            : FontWeight.w400,
                      ),
                    ),
                    if (prefs.previewLines > 0)
                      Padding(
                        padding: const EdgeInsets.only(top: 3),
                        child: Text(
                          mail.preview,
                          maxLines: prefs.previewLines,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: 12,
                            height: 1.5,
                            color: scheme.onSurfaceVariant,
                          ),
                        ),
                      ),
                    const SizedBox(height: 5),
                    Row(
                      children: [
                        if (mail.unread)
                          Padding(
                            padding: const EdgeInsets.only(right: 5),
                            child: Icon(
                              Icons.circle,
                              size: 6,
                              color: scheme.primary,
                            ),
                          ),
                        Text(
                          mail.account,
                          style: TextStyle(
                            fontSize: 10,
                            color: scheme.onSurfaceVariant,
                          ),
                        ),
                        if (mail.attachments.isNotEmpty)
                          Padding(
                            padding: const EdgeInsets.only(left: 7),
                            child: Icon(
                              Icons.attach_file,
                              size: 13,
                              color: scheme.onSurfaceVariant,
                            ),
                          ),
                      ],
                    ),
                  ],
                ),
              ),
              SizedBox(
                width: 44,
                child: Column(
                  children: [
                    IconButton(
                      tooltip: mail.starred
                          ? 'Unflag ${mail.subject}'
                          : 'Flag ${mail.subject}',
                      iconSize: 19,
                      onPressed: () => act(MailAction.star),
                      icon: Icon(
                        mail.starred ? Icons.flag : Icons.flag_outlined,
                        color: mail.starred
                            ? scheme.error
                            : scheme.onSurfaceVariant,
                      ),
                    ),
                    PopupMenuButton<MailAction>(
                      tooltip: 'Actions for ${mail.subject}',
                      icon: const Icon(Icons.more_horiz, size: 19),
                      onSelected: act,
                      itemBuilder: (_) => MailAction.values
                          .where((a) => a != MailAction.none)
                          .map(
                            (action) => PopupMenuItem(
                              value: action,
                              child: Row(
                                children: [
                                  Icon(icon(action), size: 19),
                                  const SizedBox(width: 12),
                                  Text(label(action)),
                                ],
                              ),
                            ),
                          )
                          .toList(),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
    return Semantics(
      container: true,
      label: '${mail.unread ? 'Unread' : 'Read'}, ${mail.subject}',
      child: Dismissible(
        key: ValueKey('swipe-${mail.id}'),
        direction:
            prefs.leftSwipe == MailAction.none &&
                prefs.rightSwipe == MailAction.none
            ? DismissDirection.none
            : prefs.leftSwipe == MailAction.none
            ? DismissDirection.startToEnd
            : prefs.rightSwipe == MailAction.none
            ? DismissDirection.endToStart
            : DismissDirection.horizontal,
        background: background(context, prefs.rightSwipe, false),
        secondaryBackground: background(context, prefs.leftSwipe, true),
        confirmDismiss: (direction) async {
          act(
            direction == DismissDirection.endToStart
                ? prefs.leftSwipe
                : prefs.rightSwipe,
          );
          return false;
        },
        child: row,
      ),
    );
  }
}
