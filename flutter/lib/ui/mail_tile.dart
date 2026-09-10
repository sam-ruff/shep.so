import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'format.dart';
import 'icons.dart';
import 'theme.dart';

class MailTile extends StatelessWidget {
  const MailTile({
    super.key,
    required this.mail,
    required this.workspace,
    required this.open,
    required this.act,
    this.index = 0,
  });
  final Mail mail;
  final Workspace workspace;
  final VoidCallback open;
  final void Function(MailAction) act;
  final int index;

  String label(MailAction action) => switch (action) {
    MailAction.read => mail.unread ? 'Mark read' : 'Mark unread',
    MailAction.star => mail.starred ? 'Unflag' : 'Flag',
    MailAction.select =>
      workspace.selected.contains(mail.id) ? 'Deselect' : 'Select',
    _ => action.label,
  };

  String iconName(MailAction action) => switch (action) {
    MailAction.read => mail.unread ? 'mail-open' : 'mail',
    MailAction.star => 'flag',
    _ => actionIconName(action.name),
  };

  Widget background(BuildContext context, MailAction action, bool left) {
    final destructive = action == MailAction.trash || action == MailAction.spam;
    final color = destructive
        ? ShepColors.destructive
        : ShepColors.primaryButton;
    return Container(
      color: color,
      padding: const EdgeInsets.symmetric(horizontal: 24),
      alignment: left ? Alignment.centerRight : Alignment.centerLeft,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ShepIcon(iconName(action), color: Colors.white, size: 24),
          const SizedBox(height: 5),
          Text(
            label(action),
            style: const TextStyle(
              color: Colors.white,
              fontSize: ShepText.secondary,
            ),
          ),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    final prefs = workspace.preferences;
    final selected = workspace.selected.contains(mail.id);
    final weight = mail.unread ? FontWeight.w600 : FontWeight.w400;
    final (avatarBg, avatarFg) = avatarColors(index);
    final row = Material(
      color: selected ? c.tint : c.surface,
      child: InkWell(
        onTap: workspace.selected.isNotEmpty
            ? () => act(MailAction.select)
            : open,
        onLongPress: () => act(MailAction.select),
        hoverColor: c.subtle,
        child: Container(
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: c.border)),
          ),
          padding: const EdgeInsets.fromLTRB(10, 10, 6, 10),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: SizedBox(
                  width: 12,
                  child: mail.unread
                      ? Center(
                          child: Container(
                            width: 5,
                            height: 5,
                            decoration: BoxDecoration(
                              color: c.accent,
                              shape: BoxShape.circle,
                            ),
                          ),
                        )
                      : null,
                ),
              ),
              if (prefs.avatars || selected)
                Padding(
                  padding: const EdgeInsets.only(right: 10, top: 1),
                  child: Container(
                    width: 30,
                    height: 30,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: selected ? c.tint : avatarBg,
                      shape: BoxShape.circle,
                      border: selected ? Border.all(color: c.accent) : null,
                    ),
                    child: selected
                        ? ShepIcon('check', color: c.accent, size: 16)
                        : Text(
                            avatarInitials(mail.sender),
                            style: TextStyle(
                              color: avatarFg,
                              fontSize: 11,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                  ),
                ),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Padding(
                      padding: const EdgeInsets.only(top: 6),
                      child: Row(
                        children: [
                          Expanded(
                            child: Text(
                              mail.sender,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontWeight: weight,
                                fontSize: ShepText.body,
                                color: c.text,
                              ),
                            ),
                          ),
                          if (mail.attachments.isNotEmpty)
                            Padding(
                              padding: const EdgeInsets.only(right: 6),
                              child: ShepIcon('clip', size: 13, color: c.muted),
                            ),
                          Text(
                            rowDate(mail.date),
                            style: TextStyle(
                              color: c.muted,
                              fontSize: ShepText.caption,
                            ),
                          ),
                        ],
                      ),
                    ),
                    const SizedBox(height: 5),
                    Text(
                      mail.subject,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: ShepText.body,
                        fontWeight: weight,
                        color: c.text,
                      ),
                    ),
                    if (prefs.previewLines > 0)
                      Padding(
                        padding: const EdgeInsets.only(top: 4),
                        child: Text(
                          mail.preview,
                          maxLines: prefs.previewLines,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: ShepText.secondary,
                            height: 1.45,
                            color: c.muted,
                          ),
                        ),
                      ),
                    const SizedBox(height: 4),
                    Text(
                      mail.account,
                      style: TextStyle(
                        fontSize: ShepText.caption,
                        color: c.muted,
                      ),
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
                      iconSize: 18,
                      style: mail.starred
                          ? ButtonStyle(
                              side: WidgetStatePropertyAll(
                                BorderSide(color: c.flag, width: 1.5),
                              ),
                              foregroundColor: WidgetStatePropertyAll(c.flag),
                            )
                          : null,
                      onPressed: () => act(MailAction.star),
                      icon: ShepIcon(
                        'flag',
                        color: mail.starred ? c.flag : c.muted,
                      ),
                    ),
                    PopupMenuButton<MailAction>(
                      tooltip: 'Actions for ${mail.subject}',
                      icon: const ShepIcon('more', size: 18),
                      style: IconButton.styleFrom(foregroundColor: c.muted),
                      onSelected: act,
                      itemBuilder: (_) => MailAction.values
                          .where((a) => a != MailAction.none)
                          .map(
                            (action) => PopupMenuItem(
                              value: action,
                              height: 40,
                              child: Row(
                                children: [
                                  ShepIcon(iconName(action), size: 18),
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
