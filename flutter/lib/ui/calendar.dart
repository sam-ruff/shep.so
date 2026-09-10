import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import 'icons.dart';

class CalendarView extends StatefulWidget {
  const CalendarView({super.key, required this.workspace});
  final Workspace workspace;
  @override
  State<CalendarView> createState() => _CalendarViewState();
}

class _CalendarViewState extends State<CalendarView> {
  late DateTime month =
      widget.workspace.events.firstOrNull?.start ?? DateTime.now();
  DateTime? selected;
  final months = const [
    'January',
    'February',
    'March',
    'April',
    'May',
    'June',
    'July',
    'August',
    'September',
    'October',
    'November',
    'December',
  ];
  Future<void> edit([CalendarEntry? entry]) async {
    final date = selected ?? DateTime.now();
    await showDialog<void>(
      context: context,
      builder: (_) => EventEditor(
        workspace: widget.workspace,
        entry: entry,
        date: DateTime(date.year, date.month, date.day),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final start = DateTime(month.year, month.month);
    final days = DateTime(month.year, month.month + 1, 0).day;
    final offset = start.weekday - 1;
    final entries =
        widget.workspace.events
            .where(
              (e) => selected == null
                  ? e.start.year == month.year && e.start.month == month.month
                  : DateUtils.isSameDay(e.start, selected),
            )
            .toList()
          ..sort((a, b) => a.start.compareTo(b.start));
    return ListView(
      padding: const EdgeInsets.all(18),
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                '${months[month.month - 1]} ${month.year}',
                style: Theme.of(context).textTheme.titleLarge,
              ),
            ),
            IconButton(
              tooltip: 'Previous month',
              onPressed: () => setState(() {
                month = DateTime(month.year, month.month - 1);
                selected = null;
              }),
              icon: const ShepIcon('left'),
            ),
            IconButton(
              tooltip: 'Next month',
              onPressed: () => setState(() {
                month = DateTime(month.year, month.month + 1);
                selected = null;
              }),
              icon: const ShepIcon('chevron', size: 18),
            ),
          ],
        ),
        const SizedBox(height: 16),
        Row(
          children: ['M', 'T', 'W', 'T', 'F', 'S', 'S']
              .map(
                (d) => Expanded(
                  child: Center(
                    child: Text(
                      d,
                      style: TextStyle(color: scheme.onSurfaceVariant),
                    ),
                  ),
                ),
              )
              .toList(),
        ),
        const SizedBox(height: 8),
        GridView.builder(
          shrinkWrap: true,
          physics: const NeverScrollableScrollPhysics(),
          gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
            crossAxisCount: 7,
          ),
          itemCount: ((days + offset) / 7).ceil() * 7,
          itemBuilder: (context, index) {
            final day = index - offset + 1;
            if (day < 1 || day > days) return const SizedBox();
            final date = DateTime(month.year, month.month, day);
            final has = widget.workspace.events.any(
              (e) => DateUtils.isSameDay(e.start, date),
            );
            return TextButton(
              style: TextButton.styleFrom(
                backgroundColor: DateUtils.isSameDay(date, selected)
                    ? scheme.primaryContainer
                    : null,
              ),
              onPressed: () => setState(
                () => selected = DateUtils.isSameDay(selected, date)
                    ? null
                    : date,
              ),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text('$day'),
                  SizedBox(
                    height: 5,
                    child: has
                        ? Icon(Icons.circle, size: 4, color: scheme.primary)
                        : null,
                  ),
                ],
              ),
            );
          },
        ),
        const SizedBox(height: 18),
        Row(
          children: [
            const Expanded(
              child: Text(
                'Agenda',
                style: TextStyle(fontWeight: FontWeight.w600),
              ),
            ),
            TextButton.icon(
              onPressed: edit,
              icon: const ShepIcon('plus', size: 18),
              label: const Text('New event'),
            ),
          ],
        ),
        if (entries.isEmpty)
          const Padding(
            padding: EdgeInsets.all(24),
            child: Text('No events for these dates.'),
          ),
        ...entries.map(
          (e) => Card(
            child: ListTile(
              onTap: () => edit(e),
              leading: Text(
                '${e.start.day}',
                style: TextStyle(color: scheme.primary, fontSize: 22),
              ),
              title: Text(e.title),
              subtitle: Text(
                '${e.calendar} · ${e.start.hour.toString().padLeft(2, '0')}:${e.start.minute.toString().padLeft(2, '0')}${e.location.isEmpty ? '' : ' · ${e.location}'}',
              ),
              trailing: e.readOnly
                  ? const ShepIcon('lock', size: 18)
                  : const ShepIcon('chevron', size: 18),
            ),
          ),
        ),
      ],
    );
  }
}

class EventEditor extends StatefulWidget {
  const EventEditor({
    super.key,
    required this.workspace,
    required this.date,
    this.entry,
  });
  final Workspace workspace;
  final CalendarEntry? entry;
  final DateTime date;
  @override
  State<EventEditor> createState() => _EventEditorState();
}

class _EventEditorState extends State<EventEditor> {
  late final title = TextEditingController(text: widget.entry?.title);
  late final location = TextEditingController(text: widget.entry?.location);
  String? error;
  bool saving = false;
  @override
  void dispose() {
    title.dispose();
    location.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final entry = widget.entry;
    final day = entry?.start ?? widget.date;
    return AlertDialog(
      title: Text(
        entry?.readOnly == true
            ? 'View event'
            : entry == null
            ? 'New event'
            : 'Edit event',
      ),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              entry == null
                  ? '${day.day}/${day.month}/${day.year} · All day'
                  : '${day.day}/${day.month}/${day.year} · ${day.hour.toString().padLeft(2, '0')}:${day.minute.toString().padLeft(2, '0')}',
            ),
            const SizedBox(height: 16),
            TextField(
              controller: title,
              readOnly: entry?.readOnly == true,
              decoration: const InputDecoration(labelText: 'Event title'),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: location,
              readOnly: entry?.readOnly == true,
              decoration: const InputDecoration(labelText: 'Location'),
            ),
            if (error != null) Text(error!),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: saving ? null : () => Navigator.pop(context),
          child: const Text('Cancel'),
        ),
        if (entry?.readOnly != true)
          FilledButton(
            onPressed: saving
                ? null
                : () async {
                    if (title.text.trim().isEmpty) {
                      setState(() => error = 'Enter an event title.');
                      return;
                    }
                    setState(() => saving = true);
                    final ok = await widget.workspace.saveEvent(
                      CalendarEntry(
                        entry?.id ??
                            DateTime.now().microsecondsSinceEpoch.toString(),
                        title.text.trim(),
                        day,
                        entry?.end ?? day.add(const Duration(days: 1)),
                        calendar: entry?.calendar ?? 'Personal',
                        location: location.text,
                      ),
                    );
                    if (!context.mounted) return;
                    if (ok) {
                      Navigator.pop(context);
                    } else {
                      setState(() {
                        error = widget.workspace.error;
                        saving = false;
                      });
                    }
                  },
            child: const Text('Save event'),
          ),
      ],
    );
  }
}
