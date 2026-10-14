import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/ui/words.dart';

void main() {
  // A fixed "now": Tuesday, 14 October 2026, 15:00 local time.
  final now = DateTime(2026, 10, 14, 15);
  int at(DateTime local) => local.toUtc().millisecondsSinceEpoch;

  test('the chat list says times the way people read them', () {
    expect(listTime(at(DateTime(2026, 10, 14, 9, 5)), now: now), '09:05');
    expect(listTime(at(DateTime(2026, 10, 13, 23)), now: now), 'Yesterday');
    expect(listTime(at(DateTime(2026, 10, 10, 12)), now: now), 'Sat');
    expect(listTime(at(DateTime(2026, 9, 30, 12)), now: now), 'Sep 30');
    expect(listTime(at(DateTime(2025, 12, 31, 12)), now: now), 'Dec 31, 2025');
  });

  test('day separators name the day', () {
    expect(dayLabel(at(DateTime(2026, 10, 14, 1)), now: now), 'Today');
    expect(dayLabel(at(DateTime(2026, 10, 13, 1)), now: now), 'Yesterday');
    expect(dayLabel(at(DateTime(2026, 10, 12, 1)), now: now), 'Monday, Oct 12');
    expect(
      dayLabel(at(DateTime(2025, 1, 2, 1)), now: now),
      'Thursday, Jan 2, 2025',
    );
  });

  test('times are the author’s instant in the viewer’s zone', () {
    final instant = DateTime.utc(2026, 10, 14, 12, 30);
    final local = instant.toLocal();
    final expected =
        '${local.hour.toString().padLeft(2, '0')}:'
        '${local.minute.toString().padLeft(2, '0')}';
    expect(clockTime(instant.millisecondsSinceEpoch), expected);
    expect(
      sameDay(at(DateTime(2026, 10, 14, 0, 1)), at(DateTime(2026, 10, 14, 23))),
      isTrue,
    );
  });

  test('every failure has words, and unknown ones a safe fallback', () {
    for (final code in [
      'username_taken',
      'invalid_invite',
      'unknown_account',
      'offline',
      'rate_limited',
      'unauthenticated',
    ]) {
      expect(describeFailure(code), isNot(describeFailure('???')));
    }
    expect(describeFailure('???'), 'Something went wrong. Please try again.');
  });
}
