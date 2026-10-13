import 'dart:io';

import 'package:zelamar/src/rust/api/lab.dart';

/// Starts a Crypto Lab engine.
typedef LabEngineFactory = Future<LabEngine> Function();

/// What the Crypto Lab screen needs from the core. An interface so widget
/// tests can drive the screen without the native library.
abstract interface class LabEngine {
  /// Alice and Bob, in that order.
  List<LabParticipant> get participants;

  /// Sends [text] from [from] to the other side through a real envelope.
  Future<LabExchange> send(LabSide from, String text);

  /// Reopens both devices from their encrypted stores.
  Future<void> restart();
}

/// The real lab: two `zelamar-core` devices driven over the Rust bridge.
class RustLabEngine implements LabEngine {
  new _(this._lab);

  final CryptoLab _lab;

  /// Starts a fresh lab. Its databases live in the system temp directory:
  /// the lab begins from a clean slate on every launch by design.
  static Future<LabEngine> start() async {
    final directory = [
      Directory.systemTemp.path,
      'zelamar-crypto-lab',
    ].join(Platform.pathSeparator);
    return RustLabEngine._(await CryptoLab.create(directory: directory));
  }

  @override
  List<LabParticipant> get participants => _lab.participants();

  @override
  Future<LabExchange> send(LabSide from, String text) =>
      _lab.send(from: from, text: text);

  @override
  Future<void> restart() => _lab.restart();
}
