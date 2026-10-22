const _weekdays = ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'];
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

DateTime _day(DateTime at) => DateTime(at.year, at.month, at.day);

/// "Today", "Yesterday", a weekday in the last week, then a date.
String dayLabel(DateTime at, DateTime now) {
  final days = _day(now).difference(_day(at)).inDays;
  if (days == 0) return 'Today';
  if (days == 1) return 'Yesterday';
  if (days > 1 && days < 7) return _weekdays[at.weekday - 1];
  final date = '${at.day} ${_months[at.month - 1]}';
  return at.year == now.year ? date : '$date ${at.year}';
}

bool sameDay(DateTime a, DateTime b) => _day(a) == _day(b);

String clock(DateTime at) =>
    '${at.hour.toString().padLeft(2, '0')}:${at.minute.toString().padLeft(2, '0')}';

/// "4.2 s" under a minute, then "1:05".
String seconds(int ms) {
  if (ms < 60000) return '${(ms / 1000).toStringAsFixed(1)} s';
  final total = ms ~/ 1000;
  return '${total ~/ 60}:${(total % 60).toString().padLeft(2, '0')}';
}

/// "until 14:30" today, "until tomorrow 06:00", or a date.
String until(DateTime at, DateTime now) {
  final days = _day(at).difference(_day(now)).inDays;
  if (days == 0) return 'until ${clock(at)}';
  if (days == 1) return 'until tomorrow ${clock(at)}';
  return 'until ${at.day} ${_months[at.month - 1]} ${clock(at)}';
}
