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
  @override
  void initState() {
    super.initState();
    widget.workspace.addListener(_changed);
    widget.workspace.openCalendar();
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    widget.workspace.removeListener(_changed);
    super.dispose();
  }

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
        if (widget.workspace.calendarActivities.any(
          (action) =>
              action.status != 'succeeded' && action.status != 'cancelled',
        ))
          ...widget.workspace.calendarActivities
              .where(
                (action) =>
                    action.status != 'succeeded' &&
                    action.status != 'cancelled',
              )
              .map(
                (action) => Card(
                  child: ListTile(
                    title: Text(action.statusLabel),
                    subtitle: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          action.requested.title,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                        ),
                        if (action.error case final error?) Text(error),
                      ],
                    ),
                    trailing: Wrap(
                      children: [
                        if (action.canResume)
                          TextButton(
                            onPressed: () =>
                                widget.workspace.retryCalendarActivity(action),
                            child: const Text('Retry'),
                          ),
                        if (action.canInspect)
                          TextButton(
                            onPressed: () => widget.workspace
                                .inspectCalendarActivity(action),
                            child: const Text('Check'),
                          ),
                        if (action.canCancel)
                          TextButton(
                            onPressed: () =>
                                widget.workspace.cancelCalendarActivity(action),
                            child: const Text('Cancel'),
                          ),
                      ],
                    ),
                  ),
                ),
              ),
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
              onPressed:
                  widget.workspace.calendarSources.isNotEmpty &&
                      !widget.workspace.calendarSources.any(
                        (source) => !source.readOnly,
                      )
                  ? null
                  : edit,
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
  late final eventId =
      widget.entry?.id ?? DateTime.now().microsecondsSinceEpoch.toString();
  late final title = TextEditingController(text: widget.entry?.title);
  late final location = TextEditingController(text: widget.entry?.location);
  String? error;
  bool saving = false;
  late String sourceId =
      widget.entry?.sourceId ??
      widget.workspace.calendarSources
          .where((source) => !source.readOnly)
          .firstOrNull
          ?.id ??
      'primary';
  @override
  void dispose() {
    widget.workspace.releaseCalendarEditor(sourceId, eventId);
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
              entry == null || entry.allDay
                  ? '${day.day}/${day.month}/${day.year} · All day'
                  : '${day.day}/${day.month}/${day.year} · ${day.hour.toString().padLeft(2, '0')}:${day.minute.toString().padLeft(2, '0')}',
            ),
            if (entry == null &&
                widget.workspace.calendarSources.any(
                  (source) => !source.readOnly,
                )) ...[
              const SizedBox(height: 16),
              DropdownButtonFormField<String>(
                initialValue: sourceId,
                decoration: const InputDecoration(labelText: 'Calendar'),
                items: widget.workspace.calendarSources
                    .where((source) => !source.readOnly)
                    .map(
                      (source) => DropdownMenuItem(
                        value: source.id,
                        child: Text(source.name),
                      ),
                    )
                    .toList(),
                onChanged: saving
                    ? null
                    : (value) => setState(() => sourceId = value ?? sourceId),
              ),
            ],
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
        if (entry != null && !entry.readOnly)
          TextButton(
            onPressed: saving
                ? null
                : () async {
                    setState(() => saving = true);
                    final ok = await widget.workspace.deleteEvent(entry);
                    if (!context.mounted) return;
                    if (ok) {
                      Navigator.pop(context);
                    } else {
                      setState(() {
                        saving = false;
                        error =
                            widget.workspace.error ??
                            'Event deletion could not be queued. Retry.';
                      });
                    }
                  },
            child: const Text('Delete'),
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
                        eventId,
                        title.text.trim(),
                        day,
                        entry?.end ?? day.add(const Duration(days: 1)),
                        calendar: entry?.calendar ?? 'Personal',
                        sourceId: entry?.sourceId ?? sourceId,
                        location: location.text,
                        description: entry?.description ?? '',
                        allDay: entry?.allDay ?? true,
                        etag: entry?.etag,
                        remoteUrl: entry?.remoteUrl,
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
