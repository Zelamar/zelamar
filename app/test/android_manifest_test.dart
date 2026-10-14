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

  test('release builds never back messages up to the cloud', () {
    expect(manifest, contains('android:allowBackup="false"'));
    expect(manifest, contains('@xml/data_extraction_rules'));
  });
}
