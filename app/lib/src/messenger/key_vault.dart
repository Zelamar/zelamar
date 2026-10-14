import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:zelamar/src/messenger/os_vault.dart';

/// Keeps the master key of the encrypted store sealed by the operating
/// system (Windows DPAPI, Android Keystore), never in the clear.
///
/// The sealed key sits next to the store, in the profile's directory;
/// without this user's Windows login or this phone's secure hardware it is
/// useless.
class KeyVault {
  new(this.directory, {required String profile, Sealer? sealer})
    : _sealer = sealer ?? Sealer.forPlatform(profile);

  /// The profile's directory, shared with the encrypted store.
  final String directory;
  final Sealer _sealer;

  File get _sealedKey => File('$directory${Platform.pathSeparator}key.sealed');
  File get _registered => File('$directory${Platform.pathSeparator}registered');

  /// The master key, or `null` if none was stored yet.
  ///
  /// Throws a [SealFailure] when a sealed key exists but cannot be opened.
  Future<Uint8List?> masterKey() async {
    final file = _sealedKey;
    if (!file.existsSync()) {
      return null;
    }
    return await _sealer.open(await file.readAsBytes());
  }

  /// Seals and stores [key]. The write is atomic: a crash leaves either
  /// the old file or the new one, never half of one.
  Future<void> storeMasterKey(List<int> key) async {
    await Directory(directory).create(recursive: true);
    final sealed = await _sealer.seal(key);
    final temporary = File('${_sealedKey.path}.new');
    await temporary.writeAsBytes(sealed, flush: true);
    await temporary.rename(_sealedKey.path);
  }

  /// Whether this device finished registering with the service.
  Future<bool> isRegistered() async => _registered.existsSync();

  Future<void> markRegistered() async {
    await Directory(directory).create(recursive: true);
    await _registered.writeAsString('yes', flush: true);
  }
}
