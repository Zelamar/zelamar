import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/services.dart';

/// Seals a secret with a key only this operating system user (Windows) or
/// this device's secure hardware (Android) holds. Sealed bytes are safe to
/// keep in a plain file.
abstract interface class Sealer {
  /// The sealer of the platform the app runs on.
  static Sealer forPlatform(String profile) {
    if (Platform.isWindows) {
      return const WindowsSealer();
    }
    if (Platform.isAndroid) {
      return AndroidSealer('zelamar.master.$profile');
    }
    throw UnsupportedError(
      'No secure key storage on ${Platform.operatingSystem} yet',
    );
  }

  Future<Uint8List> seal(List<int> secret);

  Future<Uint8List> open(List<int> sealed);
}

/// The sealed secret could not be opened (wrong user, wrong device, or
/// damaged).
class SealFailure implements Exception {
  const new(this.reason);

  final String reason;

  @override
  String toString() => 'SealFailure: $reason';
}

/// Android Keystore: a hardware-backed AES-256-GCM key that never leaves
/// the device (`MainActivity.kt`).
class AndroidSealer implements Sealer {
  const new(this.alias);

  /// Name of the key in the Keystore.
  final String alias;

  static const _channel = MethodChannel('zelamar/keystore');

  @override
  Future<Uint8List> seal(List<int> secret) => _call('seal', secret);

  @override
  Future<Uint8List> open(List<int> sealed) => _call('open', sealed);

  Future<Uint8List> _call(String method, List<int> data) async {
    try {
      final result = await _channel.invokeMethod<Uint8List>(method, {
        'alias': alias,
        'data': Uint8List.fromList(data),
      });
      if (result == null) {
        throw const SealFailure('the keystore returned nothing');
      }
      return result;
    } on PlatformException catch (error) {
      throw SealFailure(error.message ?? error.code);
    }
  }
}

/// A Windows `DATA_BLOB`.
final class _Blob extends Struct {
  @Uint32()
  external int size;

  external Pointer<Uint8> data;
}

typedef _ProtectNative = Int32 Function(
  Pointer<_Blob> input,
  Pointer<Utf16> description,
  Pointer<_Blob> entropy,
  Pointer<Void> reserved,
  Pointer<Void> prompt,
  Uint32 flags,
  Pointer<_Blob> output,
);
typedef _ProtectDart = int Function(
  Pointer<_Blob> input,
  Pointer<Utf16> description,
  Pointer<_Blob> entropy,
  Pointer<Void> reserved,
  Pointer<Void> prompt,
  int flags,
  Pointer<_Blob> output,
);
typedef _UnprotectNative = Int32 Function(
  Pointer<_Blob> input,
  Pointer<Pointer<Utf16>> description,
  Pointer<_Blob> entropy,
  Pointer<Void> reserved,
  Pointer<Void> prompt,
  Uint32 flags,
  Pointer<_Blob> output,
);
typedef _UnprotectDart = int Function(
  Pointer<_Blob> input,
  Pointer<Pointer<Utf16>> description,
  Pointer<_Blob> entropy,
  Pointer<Void> reserved,
  Pointer<Void> prompt,
  int flags,
  Pointer<_Blob> output,
);

/// Windows DPAPI: sealed for the signed-in Windows user, with a Zelamar
/// context so no other program's DPAPI data can be mistaken for ours.
class WindowsSealer implements Sealer {
  const new();

  static const _context = 'zelamar/master-key/v0';

  /// Never show a prompt: a background app must fail, not ask.
  static const _uiForbidden = 0x1;

  static final DynamicLibrary _crypt32 = DynamicLibrary.open('crypt32.dll');
  static final DynamicLibrary _kernel32 = DynamicLibrary.open('kernel32.dll');
  static final _ProtectDart _protect = _crypt32
      .lookupFunction<_ProtectNative, _ProtectDart>('CryptProtectData');
  static final _UnprotectDart _unprotect = _crypt32
      .lookupFunction<_UnprotectNative, _UnprotectDart>('CryptUnprotectData');
  static final Pointer<Void> Function(Pointer<Void>) _localFree = _kernel32
      .lookupFunction<
        Pointer<Void> Function(Pointer<Void>),
        Pointer<Void> Function(Pointer<Void>)
      >('LocalFree');

  @override
  Future<Uint8List> seal(List<int> secret) async => _run(secret, seal: true);

  @override
  Future<Uint8List> open(List<int> sealed) async => _run(sealed, seal: false);

  Uint8List _run(List<int> input, {required bool seal}) {
    return using((arena) {
      final inputBytes = arena<Uint8>(input.length);
      final inputView = inputBytes.asTypedList(input.length)..setAll(0, input);
      final inputBlob = arena<_Blob>();
      inputBlob.ref
        ..size = input.length
        ..data = inputBytes;

      final context = utf8.encode(_context);
      final contextBytes = arena<Uint8>(context.length);
      contextBytes.asTypedList(context.length).setAll(0, context);
      final contextBlob = arena<_Blob>();
      contextBlob.ref
        ..size = context.length
        ..data = contextBytes;

      final output = arena<_Blob>();
      try {
        final ok = seal
            ? _protect(
                inputBlob,
                nullptr,
                contextBlob,
                nullptr,
                nullptr,
                _uiForbidden,
                output,
              )
            : _unprotect(
                inputBlob,
                nullptr,
                contextBlob,
                nullptr,
                nullptr,
                _uiForbidden,
                output,
              );
        if (ok == 0) {
          throw SealFailure(
            seal
                ? 'Windows could not seal the key'
                : 'Windows could not '
                      'open the key (another user, or damaged)',
          );
        }
        final result = output.ref.data.asTypedList(output.ref.size);
        final copy = Uint8List.fromList(result);
        // Wipe Windows' copy before handing the memory back.
        result.fillRange(0, result.length, 0);
        _localFree(output.ref.data.cast());
        return copy;
      } finally {
        // Never leave the plain key behind in native memory.
        inputView.fillRange(0, inputView.length, 0);
      }
    });
  }
}
