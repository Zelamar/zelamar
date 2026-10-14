import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/version.dart';

void main() {
  test('the version shown in the app is the one in pubspec.yaml', () {
    final line = File('pubspec.yaml')
        .readAsLinesSync()
        .firstWhere((line) => line.startsWith('version:'));
    expect(line.split(' ').last.split('+').first, appVersion);
  });
}
