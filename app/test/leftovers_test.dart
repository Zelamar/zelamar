import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/messenger/leftovers.dart';

void main() {
  test('a start sweeps settled decrypted copies, and nothing else', () async {
    final temporary = await Directory.systemTemp.createTemp('zl-sweep-');
    addTearDown(() => temporary.delete(recursive: true));
    final now = DateTime.now();
    final old = now.subtract(const Duration(hours: 1));
    File at(String name, {bool settled = true}) {
      final file = File([temporary.path, name].join(Platform.pathSeparator))
        ..createSync(recursive: true);
      if (settled) {
        file.setLastModifiedSync(old);
      }
      return file;
    }

    final played = at('zelamar-play-1.m4a');
    final recorded = at('zelamar-voice-2.m4a');
    final shared = at(['share_plus', 'Plan.pdf'].join(Platform.pathSeparator));
    final playing = at('zelamar-play-3.m4a', settled: false);
    final someoneElses = at('notes.txt');

    await sweepLeftovers(temporary: temporary, now: now);

    expect(played.existsSync(), isFalse);
    expect(recorded.existsSync(), isFalse);
    expect(shared.existsSync(), isFalse);
    expect(playing.existsSync(), isTrue, reason: 'it may still be playing');
    expect(someoneElses.existsSync(), isTrue);
  });
}
