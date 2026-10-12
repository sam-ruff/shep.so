import 'mail.dart';

typedef CalendarClockTime = ({
  int hour,
  int minute,
  int second,
  int millisecond,
  int microsecond,
});

bool eventOverlapsDates(
  CalendarEntry event,
  DateTime first,
  DateTime exclusiveEnd,
) {
  if (!event.allDay) {
    return event.start.isBefore(exclusiveEnd) && event.end.isAfter(first);
  }
  DateTime nominal(DateTime value) =>
      DateTime.utc(value.year, value.month, value.day);
  final start = nominal(event.start);
  var end = nominal(event.end);
  if (!end.isAfter(start) && event.end.isAfter(event.start)) {
    end = DateTime.utc(start.year, start.month, start.day + 1);
  }
  return start.isBefore(nominal(exclusiveEnd)) && end.isAfter(nominal(first));
}

bool eventOccursOnDate(CalendarEntry event, DateTime date) =>
    eventOverlapsDates(
      event,
      DateTime(date.year, date.month, date.day),
      DateTime(date.year, date.month, date.day + 1),
    );

class EventSchedule {
  EventSchedule({required DateTime date, CalendarEntry? entry})
    : allDay = entry?.allDay ?? true,
      firstDate = _date(
        entry == null || entry.allDay
            ? entry?.start ?? date
            : entry.start.toLocal(),
      ),
      lastDate = entry == null
          ? _date(date)
          : entry.allDay
          ? _lastDate(entry)
          : _date(entry.end.toLocal()),
      from = entry == null || entry.allDay
          ? _time(DateTime(2000, 1, 1, 9))
          : _time(entry.start.toLocal()),
      to = entry == null || entry.allDay
          ? _time(DateTime(2000, 1, 1, 10))
          : _time(entry.end.toLocal());

  EventSchedule._(
    this.allDay,
    this.firstDate,
    this.lastDate,
    this.from,
    this.to,
  );
  final bool allDay;
  final DateTime firstDate, lastDate;
  final CalendarClockTime from, to;
  static DateTime _lastDate(CalendarEntry entry) {
    final last = DateTime(entry.end.year, entry.end.month, entry.end.day - 1);
    final first = _date(entry.start);
    return last.isBefore(first) ? first : last;
  }

  static DateTime _date(DateTime value) =>
      DateTime(value.year, value.month, value.day);
  static CalendarClockTime _time(DateTime value) => (
    hour: value.hour,
    minute: value.minute,
    second: value.second,
    millisecond: value.millisecond,
    microsecond: value.microsecond,
  );
  static DateTime _at(DateTime date, CalendarClockTime time) => DateTime(
    date.year,
    date.month,
    date.day,
    time.hour,
    time.minute,
    time.second,
    time.millisecond,
    time.microsecond,
  );
  DateTime get start => allDay
      ? DateTime.utc(firstDate.year, firstDate.month, firstDate.day)
      : _at(firstDate, from);
  DateTime get end => allDay
      ? DateTime.utc(lastDate.year, lastDate.month, lastDate.day + 1)
      : _at(lastDate, to);
  String? get error {
    if (!allDay &&
        (start.hour != from.hour ||
            start.minute != from.minute ||
            end.hour != to.hour ||
            end.minute != to.minute)) {
      return 'This time is unavailable on these dates. Choose another time.';
    }
    if (!end.isAfter(start)) return 'The event must end after it starts.';
    return null;
  }

  EventSchedule copy({
    bool? allDay,
    DateTime? firstDate,
    DateTime? lastDate,
    int? fromHour,
    int? fromMinute,
    int? toHour,
    int? toMinute,
  }) => EventSchedule._(
    allDay ?? this.allDay,
    _date(firstDate ?? this.firstDate),
    _date(lastDate ?? this.lastDate),
    fromHour == null
        ? from
        : _time(DateTime(2000, 1, 1, fromHour, fromMinute ?? 0)),
    toHour == null ? to : _time(DateTime(2000, 1, 1, toHour, toMinute ?? 0)),
  );
}
