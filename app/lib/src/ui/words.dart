/// Words the app says: failures in plain English, and times the way people
/// read them. Times are always the author's UTC instant shown in the
/// viewer's own time zone (protocol/content.md, rule 5).
library;

import 'package:zelamar/src/rust/api/session.dart';

/// What went wrong, in words a person can act on.
String describeFailure(String code) => switch (code) {
  'username_taken' => 'That name is taken. Try another one.',
  'reserved' => 'That name is reserved. Try another one.',
  'invalid_invite' => 'This invite code isn’t valid, or it was already used.',
  'invalid_username' ||
  'malformed' => 'Use 3–32 letters, numbers or _, starting with a letter.',
  'unknown_account' => 'No one on Zelamar goes by that name.',
  'rate_limited' => 'Too many attempts. Wait a minute, then try again.',
  'offline' => 'No internet connection.',
  'insufficient_work' ||
  'stale_request' => 'Your device’s clock looks off. Check the date and time.',
  'unauthenticated' =>
    'Something isn’t right with this account, so Zelamar stopped to keep '
        'you safe.',
  'key_missing' ||
  'wrong_key' => 'Zelamar can’t open your chats on this device.',
  'bad_link_code' =>
    'That code doesn’t look right. It has 20 letters and digits.',
  'link_taken' =>
    'Another device already used this code. Make a new one on your other '
        'device.',
  'unknown_link' =>
    'This code has expired. Make a new one on your other device.',
  'too_many_devices' =>
    'You already use Zelamar on 5 devices. Remove one first.',
  'bad_recovery_key' =>
    'That key doesn’t look right. It has 25 letters and digits.',
  'wrong_recovery_key' =>
    'That name and recovery key don’t match. Check both and try again.',
  'already_initialized' =>
    'This device already has an account, so it can’t join another one.',
  _ => 'Something went wrong. Please try again.',
};

/// A file size people read: `512 B`, `48 KB`, `2.4 MB`.
String describeSize(int bytes) {
  if (bytes < 1024) {
    return '$bytes B';
  }
  if (bytes < 1024 * 1024) {
    return '${(bytes / 1024).round()} KB';
  }
  final megabytes = bytes / (1024 * 1024);
  return '${megabytes.toStringAsFixed(megabytes < 10 ? 1 : 0)} MB';
}

/// A length people read: `0:07`, `1:23`.
String describeDuration(Duration duration) {
  final seconds = duration.inSeconds;
  return '${seconds ~/ 60}:${(seconds % 60).toString().padLeft(2, '0')}';
}

/// A file in one line, for the chat list: what the core says in
/// notifications and quotes.
String fileLine(FileView file) {
  if (file.mime.startsWith('image/')) {
    return '📷 Photo';
  }
  if (file.mime.startsWith('audio/') && file.durationMs != null) {
    return '🎤 Voice message';
  }
  return '📎 ${file.name}';
}

/// How long a call lasted, in words: `45 sec`, `12 min`, `1 h 5 min`.
/// Never `12:34`, which reads like a time of day next to one.
String describeCallLength(Duration length) {
  if (length.inSeconds < 60) {
    return '${length.inSeconds} sec';
  }
  final minutes = length.inMinutes;
  if (minutes < 60) {
    return '$minutes min';
  }
  final rest = minutes % 60;
  return rest == 0 ? '${minutes ~/ 60} h' : '${minutes ~/ 60} h $rest min';
}

/// A call in one line, as the chat shows it: `Missed call`,
/// `Video call · 12 min`.
String callLine(CallView call, {required bool outgoing}) {
  final kind = call.video ? 'video call' : 'call';
  final named = call.video ? 'Video call' : 'Call';
  final length = call.durationMs;
  return switch (call.outcome) {
    CallOutcome.ongoing => outgoing ? 'Calling…' : 'Incoming $kind',
    CallOutcome.talked =>
      length == null
          ? named
          : '$named · ${describeCallLength(Duration(milliseconds: length))}',
    CallOutcome.missed => outgoing ? 'No answer' : 'Missed $kind',
    CallOutcome.cancelled => outgoing ? 'Cancelled $kind' : 'Missed $kind',
    CallOutcome.declined => outgoing ? 'Declined' : 'You declined',
    CallOutcome.busy => outgoing ? 'Busy' : 'Missed $kind',
    CallOutcome.failed => '$named didn’t connect',
  };
}

/// Whether [call] is one the user missed: shown in the danger color.
bool missedCall(CallView call, {required bool outgoing}) =>
    !outgoing &&
    switch (call.outcome) {
      CallOutcome.missed || CallOutcome.cancelled || CallOutcome.busy => true,
      _ => false,
    };

/// A line for something that happened to a chat.
String describeEvent(String author, ChatEvent event) => switch (event.kind) {
  ChatEventKind.named => '$author named the group “${event.name ?? ''}”',
  ChatEventKind.left => '$author left',
};

const _weekdays = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
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

DateTime _local(int utcMs) =>
    DateTime.fromMillisecondsSinceEpoch(utcMs, isUtc: true).toLocal();

DateTime _day(DateTime time) => DateTime(time.year, time.month, time.day);

/// `14:05`: the time of day, 24-hour.
String clockTime(int utcMs) {
  final time = _local(utcMs);
  String two(int n) => n.toString().padLeft(2, '0');
  return '${two(time.hour)}:${two(time.minute)}';
}

/// For the chat list: `14:05` today, `Yesterday`, a weekday within a week,
/// then `Oct 13`, and `Oct 13, 2025` in other years.
String listTime(int utcMs, {DateTime? now}) {
  final time = _local(utcMs);
  final today = _day(now ?? DateTime.now());
  final days = today.difference(_day(time)).inDays;
  if (days <= 0) {
    return clockTime(utcMs);
  }
  if (days == 1) {
    return 'Yesterday';
  }
  if (days < 7) {
    return _weekdays[time.weekday - 1];
  }
  final date = '${_months[time.month - 1]} ${time.day}';
  return time.year == today.year ? date : '$date, ${time.year}';
}

/// For day separators in a chat: `Today`, `Yesterday`, `Monday, Oct 13`.
String dayLabel(int utcMs, {DateTime? now}) {
  final time = _local(utcMs);
  final today = _day(now ?? DateTime.now());
  final days = today.difference(_day(time)).inDays;
  if (days == 0) {
    return 'Today';
  }
  if (days == 1) {
    return 'Yesterday';
  }
  const long = [
    'Monday',
    'Tuesday',
    'Wednesday',
    'Thursday',
    'Friday',
    'Saturday',
    'Sunday',
  ];
  final date = '${_months[time.month - 1]} ${time.day}';
  final weekday = long[time.weekday - 1];
  return time.year == today.year
      ? '$weekday, $date'
      : '$weekday, $date, ${time.year}';
}

/// Whether two instants fall on the same local day.
bool sameDay(int aUtcMs, int bUtcMs) =>
    _day(_local(aUtcMs)) == _day(_local(bUtcMs));
