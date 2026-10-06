import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/event_schedule.dart';
import 'package:shep_mobile/model/mail.dart';

Map<String, dynamic> wire(String start, String end, {bool allDay = true}) => {
  'id': 'remote',
  'source_id': 'primary',
  'title': 'Retain',
  'start': start,
  'end': end,
  'all_day': allDay,
  'description': 'Untouched description',
  'location': 'Office',
  'etag': '"v1"',
  'remote_url': 'https://calendar.example.test/remote.ics',
};

void main() {
  test(
    'nonexistent spring-DST times are rejected rather than silently normalised',
    () {
      final zone = Platform.environment['TZ'];
      final date = zone == 'Europe/London'
          ? DateTime(2026, 3, 29)
          : DateTime(2026, 3, 8);
      final gapHour = zone == 'Europe/London' ? 1 : 2;
      final schedule = EventSchedule(date: date).copy(
        allDay: false,
        fromHour: gapHour,
        fromMinute: 30,
        toHour: 4,
        toMinute: 0,
      );
      expect(schedule.start.hour, isNot(gapHour));
      expect(
        schedule.error,
        'This time is unavailable on these dates. Choose another time.',
      );
    },
    skip: !const {
      'Europe/London',
      'America/Los_Angeles',
    }.contains(Platform.environment['TZ']),
  );
  test(
    'canonical all-day wire dates stay nominal in the configured time zone',
    () {
      final zone = Platform.environment['TZ'];
      if (zone == 'Europe/London') {
        expect(DateTime(2026, 7).timeZoneOffset > Duration.zero, isTrue);
      }
      if (zone == 'America/Los_Angeles') {
        expect(DateTime(2026, 7).timeZoneOffset < Duration.zero, isTrue);
      }
      final data = wire('2026-03-29T00:00:00.000Z', '2026-03-31T00:00:00.000Z');
      final event = CalendarEntry.fromCalendarJson(data);
      expect(event.start, DateTime.utc(2026, 3, 29));
      expect(event.end, DateTime.utc(2026, 3, 31));
      expect(event.toCalendarJson(), data);
      expect(eventOccursOnDate(event, DateTime(2026, 3, 28)), isFalse);
      expect(eventOccursOnDate(event, DateTime(2026, 3, 29)), isTrue);
      expect(eventOccursOnDate(event, DateTime(2026, 3, 30)), isTrue);
      expect(eventOccursOnDate(event, DateTime(2026, 3, 31)), isFalse);
      expect(
        eventOverlapsDates(event, DateTime(2026, 4), DateTime(2026, 5)),
        isFalse,
      );
    },
  );

  test(
    'all-day Last date remains inclusive with nominal exclusive ends across DST',
    () {
      for (final first in [
        DateTime(2026, 3, 7),
        DateTime(2026, 3, 28),
        DateTime(2026, 10, 24),
        DateTime(2026, 10, 31),
      ]) {
        final last = DateTime(first.year, first.month, first.day + 1);
        final schedule = EventSchedule(date: first).copy(lastDate: last);
        expect(schedule.error, isNull);
        expect(
          schedule.start,
          DateTime.utc(first.year, first.month, first.day),
        );
        expect(schedule.end, DateTime.utc(last.year, last.month, last.day + 1));
        expect(
          schedule.end.difference(schedule.start),
          const Duration(days: 2),
        );
        final event = CalendarEntry(
          'new',
          'Days off',
          schedule.start,
          schedule.end,
          allDay: true,
        );
        final reopened = EventSchedule(
          date: first,
          entry: CalendarEntry.fromCalendarJson(event.toCalendarJson()),
        );
        expect(reopened.firstDate, first);
        expect(reopened.lastDate, last);
      }
      if (Platform.environment['TZ'] == 'Europe/London') {
        expect(
          DateTime(2026, 3, 30).difference(DateTime(2026, 3, 28)),
          const Duration(hours: 47),
        );
      }
      if (Platform.environment['TZ'] == 'America/Los_Angeles') {
        expect(
          DateTime(2026, 3, 9).difference(DateTime(2026, 3, 7)),
          const Duration(hours: 47),
        );
      }
    },
  );

  test(
    'legacy non-midnight all-day pairs retain their exact instant encoding',
    () {
      final data = wire('2026-07-01T23:00:00.000Z', '2026-07-03T23:00:00.000Z');
      final event = CalendarEntry.fromCalendarJson(data);
      expect(event.start, DateTime.parse(data['start']).toLocal());
      expect(event.end, DateTime.parse(data['end']).toLocal());
      expect(event.toCalendarJson(), data);
      final sameDay = DateTime(
        event.start.year,
        event.start.month,
        event.start.day,
      );
      expect(eventOccursOnDate(event, sameDay), isTrue);
      expect(
        eventOccursOnDate(
          event,
          DateTime(sameDay.year, sameDay.month, sameDay.day - 1),
        ),
        isFalse,
      );
    },
  );

  test(
    'timed wire instants, sub-minute precision and untouched metadata round-trip',
    () {
      final data = wire(
        '2026-07-01T12:34:56.123456Z',
        '2026-07-02T13:45:57.234567Z',
        allDay: false,
      );
      final event = CalendarEntry.fromCalendarJson(data);
      final schedule = EventSchedule(date: event.start, entry: event);
      expect(schedule.start.toUtc(), DateTime.parse(data['start']));
      expect(schedule.end.toUtc(), DateTime.parse(data['end']));
      expect(event.toCalendarJson(), data);
      final edited = schedule.copy(
        firstDate: DateTime(2026, 7, 3),
        lastDate: DateTime(2026, 7, 4),
      );
      expect((edited.start.second, edited.start.microsecond), (56, 456));
    },
  );

  test(
    'timed and all-day invalid ranges are rejected without adjusting user dates',
    () {
      final schedule = EventSchedule(
        date: DateTime(2026, 6, 3),
      ).copy(allDay: false);
      expect(schedule.copy(toHour: 9, toMinute: 0).error, isNotNull);
      expect(schedule.copy(lastDate: DateTime(2026, 6, 2)).error, isNotNull);
      expect(
        schedule.copy(allDay: true, lastDate: DateTime(2026, 6, 2)).error,
        isNotNull,
      );
      expect(
        schedule.copy(lastDate: DateTime(2026, 6, 4), toHour: 8).error,
        isNull,
      );
    },
  );

  test(
    'time choices survive toggling all-day without elapsed-day arithmetic',
    () {
      final schedule = EventSchedule(date: DateTime(2026, 10, 24)).copy(
        allDay: false,
        lastDate: DateTime(2026, 10, 25),
        fromHour: 14,
        fromMinute: 15,
        toHour: 16,
        toMinute: 45,
      );
      final toggled = schedule.copy(allDay: true).copy(allDay: false);
      expect(toggled.start, schedule.start);
      expect(toggled.end, schedule.end);
    },
  );
}
