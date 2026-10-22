#!/usr/bin/env bash
# Push wake-ups end to end on a running emulator (ADR-0017): runs
# integration_test/push_test.dart, sends the app to the background when the
# test says so, then waits for Firebase's "New message" notification and
# takes a screenshot of it. Needs ZELAMAR_SERVICE and ZELAMAR_TEST_INVITES.
# Called by .github/workflows/push-check.yml from the app directory.
set -uo pipefail

app=com.zelamar.app
mkdir -p screenshots
log=screenshots/push-test.log
# The test leaves marker files in the app's temporary directory; its printed
# output would arrive only once it ends.
adb logcat -c

flutter test integration_test/push_test.dart -d emulator-5554 -r expanded \
  --dart-define=ZELAMAR_SERVICE="$ZELAMAR_SERVICE" \
  --dart-define=ZELAMAR_TEST_INVITES="$ZELAMAR_TEST_INVITES" >"$log" 2>&1 &
test_pid=$!

# Waits up to $2 seconds for the app to leave the marker zelamar-push-$1.
wait_for() {
  for _ in $(seq 1 "$2"); do
    adb shell run-as "$app" find . -name "zelamar-push-$1" 2>/dev/null | grep -q . && return 0
    kill -0 "$test_pid" 2>/dev/null || return 1
    sleep 1
  done
  return 1
}

found=0
if wait_for away 900; then
  echo "app is away: to the background"
  adb shell pm grant "$app" android.permission.POST_NOTIFICATIONS
  adb shell input keyevent KEYCODE_HOME
  if wait_for sent 60; then
    echo "message sent: looking for the notification"
    for _ in $(seq 1 45); do
      adb shell dumpsys notification --noredact >screenshots/notifications.txt
      if grep -q "pkg=$app" screenshots/notifications.txt &&
        grep -q 'New message' screenshots/notifications.txt; then
        found=1
        break
      fi
      sleep 2
    done
    adb shell cmd statusbar expand-notifications
    sleep 3
    adb exec-out screencap -p >screenshots/push.png
  fi
fi

wait "$test_pid"
test_status=$?
adb shell run-as "$app" find . -name 'zelamar-push-*' >screenshots/markers.txt 2>&1
adb logcat -d -s FirebaseMessaging:V FirebaseInstanceId:V >screenshots/firebase-logcat.txt
tail -n 40 "$log"
echo "notification found: $found, test status: $test_status"
[ "$found" = 1 ] && [ "$test_status" = 0 ]
