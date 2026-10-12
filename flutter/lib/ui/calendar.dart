import 'package:flutter/material.dart';
import '../model/mail.dart';
import '../model/workspace.dart';
import '../model/event_schedule.dart';
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
                  ? eventOverlapsDates(
                      e,
                      start,
                      DateTime(month.year, month.month + 1),
                    )
                  : eventOccursOnDate(e, selected!),
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
                        if (action.canAcceptCurrent)
                          TextButton(
                            onPressed: () => widget.workspace
                                .acceptCalendarCurrentState(action),
                            child: const Text('Keep current'),
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
              (e) => eventOccursOnDate(e, date),
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
                '${e.calendar} · ${e.allDay ? 'All day' : '${e.start.hour.toString().padLeft(2, '0')}:${e.start.minute.toString().padLeft(2, '0')}'}${e.location.isEmpty ? '' : ' · ${e.location}'}',
              ),
              trailing: e.readOnly || e.providerViewOnly
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
  int _formRevision = 0, _scheduleRevision = 0;
  late (String, String) _observedText;
  final Set<String> _submittedSources = {};
  bool _observedSources = false;
  late EventSchedule schedule = EventSchedule(
    date: widget.date,
    entry: widget.entry,
  );
  late String sourceId =
      widget.entry?.sourceId ??
      widget.workspace.calendarSources
          .where((source) => !source.readOnly)
          .firstOrNull
          ?.id ??
      'primary';
  bool get viewOnly {
    final current = widget.workspace.calendarSources
        .where((source) => source.id == sourceId)
        .firstOrNull;
    return widget.entry?.readOnly == true ||
        widget.entry?.providerViewOnly == true ||
        current?.readOnly == true ||
        (_observedSources && current == null);
  }

  @override
  void initState() {
    super.initState();
    _observedSources = widget.workspace.calendarSources.isNotEmpty;
    _observedText = (title.text, location.text);
    title.addListener(_textChanged);
    location.addListener(_textChanged);
    widget.workspace.addListener(_workspaceChanged);
  }

  void _workspaceChanged() {
    if (!mounted) return;
    _observedSources |= widget.workspace.calendarSources.isNotEmpty;
    setState(() {});
  }

  void _inputChanged() {
    _formRevision++;
    if (mounted) setState(() {});
  }

  void _textChanged() {
    final current = (title.text, location.text);
    if (current == _observedText) return;
    _observedText = current;
    _inputChanged();
  }

  void _changeSchedule(EventSchedule value) {
    if (viewOnly) return;
    _scheduleRevision++;
    schedule = value;
    _inputChanged();
  }

  Future<void> _pickDate({required bool first}) async {
    final chosen = await showDatePicker(
      context: context,
      initialDate: first ? schedule.firstDate : schedule.lastDate,
      firstDate: DateTime(1),
      lastDate: DateTime(9999, 12, 31),
    );
    if (!mounted || chosen == null || viewOnly) return;
    _changeSchedule(
      first
          ? schedule.copy(firstDate: chosen)
          : schedule.copy(lastDate: chosen),
    );
  }

  Future<void> _pickTime({required bool first}) async {
    final time = first ? schedule.from : schedule.to;
    final chosen = await showTimePicker(
      context: context,
      initialTime: TimeOfDay(hour: time.hour, minute: time.minute),
    );
    if (!mounted || chosen == null || viewOnly) return;
    _changeSchedule(
      first
          ? schedule.copy(fromHour: chosen.hour, fromMinute: chosen.minute)
          : schedule.copy(toHour: chosen.hour, toMinute: chosen.minute),
    );
  }

  Future<void> _save() async {
    if (saving || viewOnly) return;
    final failure = title.text.trim().isEmpty
        ? 'Enter an event title.'
        : widget.entry != null && _scheduleRevision == 0
        ? null
        : schedule.error;
    if (failure != null) {
      setState(() => error = failure);
      return;
    }
    final revision = _formRevision, entry = widget.entry;
    final retained = entry != null && _scheduleRevision == 0;
    final requested = CalendarEntry(
      eventId,
      title.text.trim(),
      retained ? entry.start : schedule.start,
      retained ? entry.end : schedule.end,
      calendar: entry?.calendar ?? 'Personal',
      sourceId: sourceId,
      location: location.text,
      description: entry?.description ?? '',
      allDay: retained ? entry.allDay : schedule.allDay,
      etag: entry?.etag,
      remoteUrl: entry?.remoteUrl,
    );
    setState(() {
      saving = true;
      error = null;
    });
    _submittedSources.add(sourceId);
    bool ok;
    try {
      ok = await widget.workspace.saveEvent(requested);
    } catch (_) {
      if (!mounted) return;
      setState(() {
        saving = false;
        error =
            'Could not confirm whether the event was queued. Keep this form open and retry.';
      });
      return;
    }
    if (!mounted) return;
    if (ok && revision == _formRevision) {
      Navigator.pop(context);
      return;
    }
    setState(() {
      saving = false;
      error = ok
          ? 'The earlier version was queued. Your newer edits are still open.'
          : widget.workspace.error ??
                'The event could not be queued. Keep this form open and retry.';
    });
  }

  Widget _scheduleButton(String label, String value, VoidCallback? choose) =>
      Padding(
        padding: const EdgeInsets.only(bottom: 8),
        child: OutlinedButton(
          onPressed: choose,
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 12),
            child: SizedBox(
              width: double.infinity,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [Text(label), const SizedBox(height: 4), Text(value)],
              ),
            ),
          ),
        ),
      );
  Widget _schedulePair(Widget first, Widget last) => Row(
    children: [
      Expanded(child: first),
      const SizedBox(width: 8),
      Expanded(child: last),
    ],
  );
  @override
  void dispose() {
    widget.workspace.removeListener(_workspaceChanged);
    for (final source in {..._submittedSources, sourceId}) {
      widget.workspace.releaseCalendarEditor(source, eventId);
    }
    title.dispose();
    location.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final entry = widget.entry;
    final viewOnly = this.viewOnly;
    return AlertDialog(
      title: Text(
        viewOnly
            ? 'View event'
            : entry == null
            ? 'New event'
            : 'Edit event',
      ),
      content: SizedBox(
        width: 360,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                '${schedule.firstDate.day}/${schedule.firstDate.month}/${schedule.firstDate.year} · ${schedule.allDay ? 'All day' : '${schedule.from.hour.toString().padLeft(2, '0')}:${schedule.from.minute.toString().padLeft(2, '0')}'}',
              ),
              if (entry?.providerViewOnly == true) ...[
                const SizedBox(height: 12),
                const Text(
                  'Recurring CalDAV events are view-only because this device cannot safely change a single occurrence.',
                ),
              ],
              if (viewOnly && entry?.providerViewOnly != true)
                const Text(
                  'This calendar is read-only or no longer available.',
                ),
              if (entry == null &&
                  widget.workspace.calendarSources.any(
                    (source) => !source.readOnly,
                  )) ...[
                const SizedBox(height: 16),
                DropdownButtonFormField<String>(
                  icon: const ShepIcon('chevron-down', size: 18),
                  initialValue:
                      widget.workspace.calendarSources.any(
                        (source) => source.id == sourceId,
                      )
                      ? sourceId
                      : null,
                  decoration: InputDecoration(
                    labelText: 'Calendar',
                    helperText: _submittedSources.isEmpty
                        ? null
                        : 'This save stays with this calendar.',
                  ),
                  items: widget.workspace.calendarSources
                      .map(
                        (source) => DropdownMenuItem(
                          value: source.id,
                          enabled: !source.readOnly,
                          child: Text(source.name),
                        ),
                      )
                      .toList(),
                  onChanged: saving || _submittedSources.isNotEmpty
                      ? null
                      : (value) {
                          if (_submittedSources.isNotEmpty) return;
                          sourceId = value ?? sourceId;
                          _inputChanged();
                        },
                ),
              ],
              const SizedBox(height: 16),
              TextField(
                controller: title,
                readOnly: viewOnly,
                decoration: const InputDecoration(labelText: 'Event title'),
              ),
              SwitchListTile.adaptive(
                contentPadding: EdgeInsets.zero,
                title: const Text('All day'),
                value: schedule.allDay,
                onChanged: viewOnly
                    ? null
                    : (value) => _changeSchedule(schedule.copy(allDay: value)),
              ),
              _schedulePair(
                _scheduleButton(
                  'Start date',
                  '${schedule.firstDate.day}/${schedule.firstDate.month}/${schedule.firstDate.year}',
                  viewOnly ? null : () => _pickDate(first: true),
                ),
                _scheduleButton(
                  'Last date',
                  '${schedule.lastDate.day}/${schedule.lastDate.month}/${schedule.lastDate.year}',
                  viewOnly ? null : () => _pickDate(first: false),
                ),
              ),
              if (!schedule.allDay) ...[
                _schedulePair(
                  _scheduleButton(
                    'From',
                    '${schedule.from.hour.toString().padLeft(2, '0')}:${schedule.from.minute.toString().padLeft(2, '0')}',
                    viewOnly ? null : () => _pickTime(first: true),
                  ),
                  _scheduleButton(
                    'To',
                    '${schedule.to.hour.toString().padLeft(2, '0')}:${schedule.to.minute.toString().padLeft(2, '0')}',
                    viewOnly ? null : () => _pickTime(first: false),
                  ),
                ),
                const Text('Times use this device’s local time zone.'),
              ],
              const SizedBox(height: 16),
              TextField(
                controller: location,
                readOnly: viewOnly,
                decoration: const InputDecoration(labelText: 'Location'),
              ),
              if (error != null)
                Semantics(
                  liveRegion: true,
                  child: Text(
                    error!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              if (_submittedSources.isNotEmpty)
                const Text(
                  'Queued events remain in Calendar when this form closes.',
                ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(
            _submittedSources.isEmpty && !saving ? 'Cancel' : 'Close',
          ),
        ),
        if (entry != null && !viewOnly)
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
        if (!viewOnly)
          FilledButton(
            onPressed: saving ? null : _save,
            child: const Text('Save event'),
          ),
      ],
    );
  }
}
