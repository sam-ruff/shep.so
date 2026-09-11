import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/mail_selection.dart';
import '../model/workspace.dart';
import 'format.dart';
import 'icons.dart';
import 'theme.dart';

/// Desktop-style checkbox: 16px, 4px radius, subtle border, accent fill.
class ShepCheckbox extends StatelessWidget {
  const ShepCheckbox({
    super.key,
    required this.value,
    required this.onChanged,
    required this.label,
  });
  final bool value;
  final ValueChanged<bool>? onChanged;
  final String label;

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    final dark = Theme.of(context).brightness == Brightness.dark;
    return Semantics(
      label: label,
      checked: value,
      enabled: onChanged != null,
      child: InkWell(
        onTap: onChanged == null ? null : () => onChanged!(!value),
        borderRadius: BorderRadius.circular(ShepRadius.control),
        child: SizedBox(
          width: 40,
          height: 40,
          child: Center(
            child: Container(
              width: 16,
              height: 16,
              decoration: BoxDecoration(
                color: value ? c.accent : c.surface,
                border: Border.all(color: value ? c.accent : c.border),
                borderRadius: BorderRadius.circular(4),
              ),
              child: value
                  ? ShepIcon(
                      'check',
                      size: 12,
                      color: dark ? c.bg : Colors.white,
                    )
                  : null,
            ),
          ),
        ),
      ),
    );
  }
}

class MailTile extends StatefulWidget {
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

  @override
  State<MailTile> createState() => _MailTileState();
}

class _MailTileState extends State<MailTile> {
  Mail get mail => widget.mail;
  Workspace get workspace => widget.workspace;
  MailSelection? get selection => workspace.selection;
  String? _watched;

  @override
  void initState() {
    super.initState();
    _watch();
  }

  @override
  void didUpdateWidget(MailTile old) {
    super.didUpdateWidget(old);
    if (old.mail.id != mail.id) {
      _unwatch();
      _watch();
    }
  }

  void _watch() {
    // Rendered rows register as observations; nothing reads the message body.
    selection?.watch(mail.id);
    _watched = mail.id;
  }

  void _unwatch() {
    if (_watched case final id?) selection?.unwatch(id);
    _watched = null;
  }

  @override
  void dispose() {
    _unwatch();
    super.dispose();
  }

  String label(MailAction action) => switch (action) {
    MailAction.read => mail.unread ? 'Mark read' : 'Mark unread',
    MailAction.star => mail.starred ? 'Unflag' : 'Flag',
    MailAction.select => selected ? 'Deselect' : 'Select',
    _ => action.label,
  };

  String iconName(MailAction action) => switch (action) {
    MailAction.read => mail.unread ? 'mail-open' : 'mail',
    MailAction.star => 'flag',
    _ => actionIconName(action.name),
  };

  bool get selecting => selection?.mode ?? false;
  bool get selected => selection?.selected(mail.id) ?? false;

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
    final selecting = this.selecting;
    final selected = this.selected;
    final weight = mail.unread ? FontWeight.w600 : FontWeight.w400;
    final (avatarBg, avatarFg) = avatarColors(widget.index);
    final hasAnchor = selection?.anchor != null && selection?.anchor != mail.id;
    final row = Material(
      color: selected ? c.tint : c.surface,
      child: InkWell(
        onTap: selecting ? () => selection?.toggle(mail.id) : widget.open,
        // Long press enters selection; in selection mode it extends the range
        // from the anchor, matching the desktop Shift+click.
        onLongPress: () => selecting && hasAnchor
            ? selection?.range(mail.id, additive: true)
            : selection?.toggle(mail.id),
        hoverColor: c.subtle,
        child: Container(
          decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: c.border)),
          ),
          padding: const EdgeInsets.fromLTRB(4, 10, 6, 10),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (selecting)
                Padding(
                  padding: const EdgeInsets.only(top: 2),
                  child: ShepCheckbox(
                    value: selected,
                    label:
                        '${selected ? 'Deselect' : 'Select'} ${mail.subject}',
                    onChanged: (_) => selection?.toggle(mail.id),
                  ),
                )
              else
                Padding(
                  padding: const EdgeInsets.only(top: 12, left: 6),
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
              if (prefs.avatars && !selecting)
                Padding(
                  padding: const EdgeInsets.only(right: 10, top: 1),
                  child: Container(
                    width: 30,
                    height: 30,
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: avatarBg,
                      shape: BoxShape.circle,
                    ),
                    child: Text(
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
                      onPressed: () => widget.act(MailAction.star),
                      icon: ShepIcon(
                        'flag',
                        color: mail.starred ? c.flag : c.muted,
                      ),
                    ),
                    PopupMenuButton<MailAction>(
                      tooltip: 'Actions for ${mail.subject}',
                      icon: const ShepIcon('more', size: 18),
                      style: IconButton.styleFrom(foregroundColor: c.muted),
                      onSelected: (action) {
                        if (action == MailAction.none) {
                          selection?.range(mail.id, additive: true);
                          return;
                        }
                        widget.act(action);
                      },
                      itemBuilder: (_) => [
                        for (final action in MailAction.values.where(
                          (a) => a != MailAction.none,
                        ))
                          PopupMenuItem(
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
                        // Visible equivalent of the long-press range gesture.
                        if (selecting && hasAnchor)
                          const PopupMenuItem(
                            value: MailAction.none,
                            height: 40,
                            child: Row(
                              children: [
                                ShepIcon('check-circle', size: 18),
                                SizedBox(width: 12),
                                Text('Select up to here'),
                              ],
                            ),
                          ),
                      ],
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
      label:
          '${mail.unread ? 'Unread' : 'Read'}, ${mail.subject}'
          '${selecting ? (selected ? ', selected' : ', not selected') : ''}',
      child: Dismissible(
        key: ValueKey('swipe-${mail.id}'),
        direction:
            selecting ||
                (prefs.leftSwipe == MailAction.none &&
                    prefs.rightSwipe == MailAction.none)
            ? DismissDirection.none
            : prefs.leftSwipe == MailAction.none
            ? DismissDirection.startToEnd
            : prefs.rightSwipe == MailAction.none
            ? DismissDirection.endToStart
            : DismissDirection.horizontal,
        background: background(context, prefs.rightSwipe, false),
        secondaryBackground: background(context, prefs.leftSwipe, true),
        confirmDismiss: (direction) async {
          widget.act(
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
