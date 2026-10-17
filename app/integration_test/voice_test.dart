import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/ui/chat/voice.dart';

/// Records from the real microphone, so it needs one: run it by hand with
///
///     flutter test integration_test/voice_test.dart -d windows \
///       --dart-define=ZELAMAR_VOICE_TEST=true
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  const enabled = bool.fromEnvironment('ZELAMAR_VOICE_TEST');

  test('a voice message is recorded as AAC in an MP4 file', () async {
    final recorder = VoiceRecorder();
    addTearDown(recorder.dispose);
    expect(await recorder.start(), isTrue, reason: 'microphone allowed');
    await Future<void>.delayed(const Duration(milliseconds: 1500));
    final voice = await recorder.finish();
    expect(voice, isNotNull);
    expect(voice!.mime, 'audio/mp4');
    expect(voice.durationMs, greaterThanOrEqualTo(1400));
    expect(voice.bytes.length, greaterThan(1000));
    expect(ascii.decode(voice.bytes.sublist(4, 8)), 'ftyp');
  }, skip: enabled ? false : 'needs a microphone');
}
