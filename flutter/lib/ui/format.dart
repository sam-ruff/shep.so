const _months = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

String _two(int value) => value.toString().padLeft(2, '0');

String clockTime(DateTime date) => '${_two(date.hour)}:${_two(date.minute)}';

String dayMonth(DateTime date) =>
    '${_two(date.day)} ${_months[date.month - 1]}';

/// Desktop mail-row date: the time today, otherwise day and month.
String rowDate(DateTime date, {DateTime? now}) {
  final local = date.toLocal();
  final today = (now ?? DateTime.now()).toLocal();
  final sameDay =
      local.year == today.year &&
      local.month == today.month &&
      local.day == today.day;
  return sameDay ? clockTime(local) : dayMonth(local);
}

/// Desktop reader date: "09 Sep 2026" over the time.
String readerDate(DateTime date) {
  final local = date.toLocal();
  return '${dayMonth(local)} ${local.year}\n${clockTime(local)}';
}
