import 'package:flutter/material.dart';
import 'icons.dart';
import 'theme.dart';

/// Desktop `settings_card`: bordered surface with the title inside.
class SettingsCard extends StatelessWidget {
  const SettingsCard({
    super.key,
    required this.title,
    required this.children,
    this.description,
  });
  final String title;
  final String? description;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    // Each row keeps its own semantics node instead of merging the whole card.
    return Card(
      semanticContainer: false,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(0, 20, 0, 6),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
              child: Text(
                title,
                style: TextStyle(
                  fontSize: ShepText.cardTitle,
                  fontWeight: FontWeight.w600,
                  color: c.text,
                ),
              ),
            ),
            if (description case final text?)
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
                child: Text(
                  text,
                  style: TextStyle(
                    fontSize: ShepText.secondary,
                    color: c.muted,
                  ),
                ),
              ),
            ...children,
          ],
        ),
      ),
    );
  }
}

/// Desktop pick-list chevron for dropdown form fields, kept at 16px even
/// when the dropdown hands the icon slot tight 24px constraints.
Widget dropdownChevron() => const Align(
  widthFactor: 1,
  heightFactor: 1,
  child: ShepIcon('chevron-down', size: 16),
);

/// Desktop `pick_list`: a bordered dropdown with a chevron and 12px text.
Widget pickList<T>(
  BuildContext context, {
  required T? value,
  required List<DropdownMenuItem<T>> items,
  required ValueChanged<T?>? onChanged,
}) {
  final c = ShepColors.of(context);
  return Container(
    constraints: const BoxConstraints(minHeight: 40),
    padding: const EdgeInsets.only(left: 10, right: 6),
    decoration: BoxDecoration(
      color: c.surface,
      border: Border.all(color: c.border),
      borderRadius: BorderRadius.circular(ShepRadius.ghost),
    ),
    child: DropdownButton<T>(
      value: value,
      items: items,
      onChanged: onChanged,
      underline: const SizedBox(),
      isDense: true,
      icon: Padding(
        padding: const EdgeInsets.only(left: 8),
        child: ShepIcon('chevron-down', size: 16, color: c.muted),
      ),
      style: TextStyle(
        fontSize: ShepText.secondary,
        color: c.text,
        fontFamily: 'NotoSans',
      ),
      dropdownColor: c.surface,
      borderRadius: BorderRadius.circular(ShepRadius.control),
      focusColor: Colors.transparent,
    ),
  );
}

/// Desktop notice bar: full-width strip with quiet text and a close control.
class NoticeBar extends StatelessWidget {
  const NoticeBar({
    super.key,
    required this.child,
    this.error = false,
    this.trailing = const [],
  });
  final Widget child;
  final bool error;
  final List<Widget> trailing;

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    return Material(
      color: error ? c.errorSurface : c.tint,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 6, 6, 6),
        child: Row(
          children: [
            Expanded(
              child: DefaultTextStyle.merge(
                style: TextStyle(fontSize: ShepText.body, color: c.text),
                child: child,
              ),
            ),
            ...trailing,
          ],
        ),
      ),
    );
  }
}

/// Desktop toast: bordered card with a check mark and optional actions.
class ToastCard extends StatelessWidget {
  const ToastCard({super.key, required this.child, this.trailing = const []});
  final Widget child;
  final List<Widget> trailing;

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
      child: Material(
        color: c.surface,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(ShepRadius.card),
          side: BorderSide(color: c.border),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(14, 4, 4, 4),
          child: Row(
            children: [
              ShepIcon('check', size: 16, color: c.muted),
              const SizedBox(width: 10),
              Expanded(
                child: DefaultTextStyle.merge(
                  style: TextStyle(fontSize: ShepText.body, color: c.text),
                  child: child,
                ),
              ),
              ...trailing,
            ],
          ),
        ),
      ),
    );
  }
}

/// Desktop empty state: quiet icon, semibold title and muted copy.
class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.icon,
    required this.title,
    required this.message,
    this.iconSize = 26,
    this.titleSize = ShepText.reader,
  });
  final String icon;
  final String title;
  final String message;
  final double iconSize;
  final double titleSize;

  @override
  Widget build(BuildContext context) {
    final c = ShepColors.of(context);
    return Center(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 15, vertical: 50),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ShepIcon(icon, size: iconSize, color: c.muted),
            const SizedBox(height: 14),
            Text(
              title,
              textAlign: TextAlign.center,
              style: TextStyle(
                fontSize: titleSize,
                fontWeight: FontWeight.w600,
                color: c.text,
              ),
            ),
            const SizedBox(height: 14),
            Text(
              message,
              textAlign: TextAlign.center,
              style: TextStyle(fontSize: ShepText.secondary, color: c.muted),
            ),
          ],
        ),
      ),
    );
  }
}
