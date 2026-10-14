import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/os_vault.dart';

/// Reverses the bytes: enough to tell sealed from plain in tests.
class _MirrorSealer implements Sealer {
  int sealed = 0;

  @override
  Future<Uint8List> seal(List<int> secret) async {
    sealed++;
    return Uint8List.fromList(secret.reversed.toList());
  }

  @override
  Future<Uint8List> open(List<int> sealed) async =>
      Uint8List.fromList(sealed.reversed.toList());
}

void main() {
  late Directory directory;
  setUp(() async => directory = await Directory.systemTemp.createTemp('kv-'));
  tearDown(() => directory.delete(recursive: true));

  test('stores only the sealed key, and gives the plain key back', () async {
    final sealer = _MirrorSealer();
    final path = '${directory.path}${Platform.pathSeparator}profile';
    final vault = KeyVault(path, profile: 'test', sealer: sealer);
    expect(await vault.masterKey(), isNull);

    final key = List<int>.generate(32, (i) => i);
    await vault.storeMasterKey(key);
    expect(sealer.sealed, 1);
    final onDisk = File('$path${Platform.pathSeparator}key.sealed');
    expect(onDisk.readAsBytesSync(), key.reversed.toList());
    expect(
      File('${onDisk.path}.new').existsSync(),
      isFalse,
      reason: 'the write is atomic',
    );
    expect(await vault.masterKey(), key);

    expect(await vault.isRegistered(), isFalse);
    await vault.markRegistered();
    expect(await vault.isRegistered(), isTrue);
  });

  test('Windows DPAPI seals for this user, with Zelamar context', () async {
    const sealer = WindowsSealer();
    final key = Uint8List.fromList(List<int>.generate(32, (i) => 255 - i));
    final sealed = await sealer.seal(key);
    expect(sealed, isNot(contains(key)));
    expect(sealed.length, greaterThan(key.length));
    expect(await sealer.open(sealed), key);

    final damaged = Uint8List.fromList(sealed)..[sealed.length - 1] ^= 1;
    await expectLater(sealer.open(damaged), throwsA(isA<SealFailure>()));
  }, skip: !Platform.isWindows);
}
