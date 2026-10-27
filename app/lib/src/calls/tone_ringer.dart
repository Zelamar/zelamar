import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:audioplayers/audioplayers.dart';
import 'package:path_provider/path_provider.dart';
import 'package:zelamar/src/calls/call_controller.dart';

/// Rings with tones made right here: no sound files to license or ship.
///
/// - The ring is a soft two-note chime, G5 then C6, every two seconds.
/// - The ring-back, while the other phone rings, is one low, quiet tone
///   every four seconds.
class ToneRinger implements Ringer {
  AudioPlayer? _player;
  int _generation = 0;

  @override
  void ring() => unawaited(_play('zelamar-tone-ring.wav', _chime, 0.9));

  @override
  void ringBack() =>
      unawaited(_play('zelamar-tone-ringback.wav', _ringBack, 0.35));

  @override
  void stop() {
    _generation++;
    final player = _player;
    _player = null;
    unawaited(
      _quietly(() async {
        await player?.stop();
        await player?.dispose();
      }),
    );
  }

  Future<void> _play(String name, Uint8List Function() make, double volume) {
    stop();
    final generation = _generation;
    return _quietly(() async {
      final file = File(
        '${(await getTemporaryDirectory()).path}${Platform.pathSeparator}$name',
      );
      if (!file.existsSync()) {
        await file.writeAsBytes(make(), flush: true);
      }
      if (generation != _generation) {
        return;
      }
      final player = AudioPlayer();
      _player = player;
      await player.setReleaseMode(ReleaseMode.loop);
      await player.setVolume(volume);
      await player.play(DeviceFileSource(file.path));
      if (generation != _generation) {
        await player.stop();
        await player.dispose();
      }
    });
  }
}

const _rate = 22050;

/// G5 then C6, each fading like a struck bell, then quiet: two seconds.
Uint8List _chime() => _wave(2, (t) {
  double bell(double start, double frequency) {
    final local = t - start;
    if (local < 0 || local > 0.9) {
      return 0;
    }
    final attack = min(1, local / 0.008);
    return attack * exp(-local * 4.5) * sin(2 * pi * frequency * local);
  }

  return 0.45 * bell(0, 783.99) + 0.45 * bell(0.32, 1046.5);
});

/// A warm 425 Hz tone for one second, then three seconds of quiet.
Uint8List _ringBack() => _wave(4, (t) {
  if (t > 1.0) {
    return 0;
  }
  final edge = min(1, min(t, 1.0 - t) / 0.03);
  return 0.5 * edge * sin(2 * pi * 425 * t);
});

/// A mono 16-bit PCM WAV file of [seconds], sampling [at] (from -1 to 1).
Uint8List _wave(double seconds, double Function(double t) at) {
  final samples = (seconds * _rate).round();
  final bytes = ByteData(44 + samples * 2);
  void text(int offset, String value) {
    for (var i = 0; i < value.length; i++) {
      bytes.setUint8(offset + i, value.codeUnitAt(i));
    }
  }

  text(0, 'RIFF');
  bytes.setUint32(4, 36 + samples * 2, Endian.little);
  text(8, 'WAVE');
  text(12, 'fmt ');
  bytes
    ..setUint32(16, 16, Endian.little)
    ..setUint16(20, 1, Endian.little)
    ..setUint16(22, 1, Endian.little)
    ..setUint32(24, _rate, Endian.little)
    ..setUint32(28, _rate * 2, Endian.little)
    ..setUint16(32, 2, Endian.little)
    ..setUint16(34, 16, Endian.little);
  text(36, 'data');
  bytes.setUint32(40, samples * 2, Endian.little);
  for (var i = 0; i < samples; i++) {
    final value = (at(i / _rate).clamp(-1.0, 1.0) * 32767).round();
    bytes.setInt16(44 + i * 2, value, Endian.little);
  }
  return bytes.buffer.asUint8List();
}

Future<void> _quietly(Future<void> Function() work) async {
  try {
    await work();
  } on Object {
    // A ring that cannot sound is not worth failing a call for.
  }
}
