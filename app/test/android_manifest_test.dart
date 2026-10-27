import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

/// The release manifest is the one users get: debug and profile builds add
/// INTERNET on their own, so tests on an emulator never notice it missing.
void main() {
  final manifest = File('android/app/src/main/AndroidManifest.xml')
      .readAsStringSync();

  test('release builds may use the network', () {
    expect(manifest, contains('android.permission.INTERNET'));
  });

  test('voice messages may use the microphone', () {
    expect(manifest, contains('android.permission.RECORD_AUDIO'));
  });

  test('calls may use the microphone, camera, speaker and network state', () {
    for (final permission in [
      'RECORD_AUDIO',
      'CAMERA',
      'MODIFY_AUDIO_SETTINGS',
      'ACCESS_NETWORK_STATE',
    ]) {
      expect(manifest, contains('android.permission.$permission'));
    }
    expect(
      manifest,
      contains(
        '<uses-feature android:name="android.hardware.camera" '
        'android:required="false" />',
      ),
      reason: 'phones without a camera can still install Zelamar',
    );
  });

  test('a call rings a closed phone and keeps its microphone', () {
    for (final permission in [
      'USE_FULL_SCREEN_INTENT',
      'FOREGROUND_SERVICE',
      'FOREGROUND_SERVICE_MICROPHONE',
      'FOREGROUND_SERVICE_CAMERA',
    ]) {
      expect(manifest, contains('android.permission.$permission'));
    }
    expect(manifest, contains('android:name=".ZelamarMessagingService"'));
    expect(manifest, contains('com.google.firebase.MESSAGING_EVENT'));
    expect(
      manifest,
      contains('android:foregroundServiceType="microphone|camera"'),
    );
    expect(manifest, contains('android:name=".CallActionReceiver"'));
    expect(
      RegExp('android:exported="true"').allMatches(manifest).length,
      1,
      reason: 'only the launcher is reachable from other apps',
    );
  });

  test('release builds never back messages up to the cloud', () {
    expect(manifest, contains('android:allowBackup="false"'));
    expect(manifest, contains('@xml/data_extraction_rules'));
  });
}
