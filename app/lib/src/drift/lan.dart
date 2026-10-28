import 'dart:async';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// What nearby delivery needs from the messenger (ADR-0020).
abstract interface class LanHost {
  /// The envelopes that may go to neighbours, tagged under [nonce].
  Future<List<LanParcel>> offer(Uint8List nonce);

  /// This device's mailbox tag under [nonce].
  Uint8List ownTag(Uint8List nonce);

  /// Opens an envelope a neighbour handed over.
  Future<void> receive(Uint8List envelope);
}

/// Frame kinds of the exchange (`protocol/drift-lan.md`).
abstract final class _Kind {
  static const hello = 1;
  static const ask = 2;
  static const have = 3;
  static const envelope = 4;
  static const taken = 5;
  static const done = 6;
}

const _magic = [0x5A, 0x4C];
const _version = 0;
const _idLength = 16;
const _tagLength = 16;

/// Largest frame: one envelope (55 bytes of header, 1 MiB of payload).
const int _maxFrame = 55 + (1 << 20);

/// Most mailboxes one exchange asks about, and envelopes it hands over.
const _maxTags = 255;
const _maxEnvelopes = 256;

/// Drift D1 (ADR-0020): finds other Zelamar devices on the same local
/// network by a beacon that says nothing about anyone, and hands each one
/// the envelopes for the mailboxes it turns out to host.
class LanNode {
  new(
    this.host, {
    this.group = '239.77.65.82',
    this.port = 47321,
    this.beaconEvery = const Duration(seconds: 3),
    this.retryEvery = const Duration(seconds: 10),
    this.timeout = const Duration(seconds: 10),
    Random? random,
  }) : _random = random ?? Random.secure();

  final LanHost host;

  /// Where beacons go: a multicast group and port.
  final String group;
  final int port;
  final Duration beaconEvery;

  /// How soon the same neighbour is offered envelopes again.
  final Duration retryEvery;

  /// How long one step of an exchange may take.
  final Duration timeout;
  final Random _random;

  /// Called when [neighbours] changes.
  void Function()? onChanged;

  RawDatagramSocket? _beacons;
  ServerSocket? _server;
  Timer? _beat;
  Uint8List _session = Uint8List(0);

  /// Neighbours by session id: where they listen, and when they were last
  /// heard.
  final _seen = <String, _Neighbour>{};

  bool get running => _server != null;

  /// How many neighbours were heard lately.
  int get neighbours {
    final now = DateTime.now();
    return _seen.values
        .where((n) => now.difference(n.heard) < beaconEvery * 4)
        .length;
  }

  /// The port this device listens on while [running].
  int? get listening => _server?.port;

  /// Starts listening and, with [announce], sending beacons, under a new
  /// session id.
  Future<void> start({bool announce = true}) async {
    if (running) {
      return;
    }
    _session = _bytes(_idLength);
    final server = await ServerSocket.bind(InternetAddress.anyIPv4, 0);
    _server = server;
    server.listen((socket) => unawaited(_serve(socket)));
    if (!announce) {
      return;
    }
    final beacons = await RawDatagramSocket.bind(
      InternetAddress.anyIPv4,
      port,
      reusePort: !Platform.isWindows && !Platform.isAndroid,
    );
    _beacons = beacons;
    beacons
      ..multicastLoopback = true
      ..joinMulticast(InternetAddress(group))
      ..listen((event) {
        if (event == RawSocketEvent.read) {
          final datagram = beacons.receive();
          if (datagram != null) {
            _heard(datagram);
          }
        }
      });
    _beat = Timer.periodic(beaconEvery, (_) => _beacon());
    _beacon();
  }

  /// Stops: no beacons, no listening, neighbours forgotten.
  Future<void> stop() async {
    _beat?.cancel();
    _beat = null;
    _beacons?.close();
    _beacons = null;
    await _server?.close();
    _server = null;
    final had = _seen.isNotEmpty;
    _seen.clear();
    if (had) {
      onChanged?.call();
    }
  }

  /// Offers what waits to every neighbour now, as after a new message.
  void push() {
    for (final neighbour in _seen.values) {
      neighbour.offered = null;
    }
    for (final entry in _seen.entries.toList()) {
      unawaited(_offerTo(entry.key, entry.value));
    }
  }

  /// Hands the device listening at [address] and [port] what is waiting
  /// for it, as if its beacon had just been heard.
  @visibleForTesting
  Future<void> handTo(InternetAddress address, int port) => _offerTo(
    'direct',
    _seen.putIfAbsent('direct', () => _Neighbour(address, port)),
  );

  void _beacon() {
    final server = _server;
    final beacons = _beacons;
    if (server == null || beacons == null) {
      return;
    }
    final beacon = BytesBuilder()
      ..add(_magic)
      ..addByte(_version)
      ..add(_session)
      ..add([server.port >> 8, server.port & 0xFF]);
    try {
      beacons.send(beacon.toBytes(), InternetAddress(group), port);
    } on SocketException {
      // No network right now; the next beat tries again.
    }
  }

  void _heard(Datagram datagram) {
    final data = datagram.data;
    if (data.length != 2 + 1 + _idLength + 2 ||
        data[0] != _magic[0] ||
        data[1] != _magic[1] ||
        data[2] != _version) {
      return;
    }
    final session = data.sublist(3, 3 + _idLength);
    if (_same(session, _session)) {
      return;
    }
    final key = _hex(session);
    final tcp = (data[19] << 8) | data[20];
    final neighbour = _seen[key];
    final isNew = neighbour == null;
    final known = (neighbour ?? _Neighbour(datagram.address, tcp))
      ..address = datagram.address
      ..port = tcp
      ..heard = DateTime.now();
    _seen[key] = known;
    if (isNew) {
      onChanged?.call();
    }
    unawaited(_offerTo(key, known));
  }

  /// Hands [neighbour] the envelopes for its mailboxes, unless it was just
  /// offered them.
  Future<void> _offerTo(String key, _Neighbour neighbour) async {
    final last = neighbour.offered;
    if (neighbour.busy ||
        (last != null && DateTime.now().difference(last) < retryEvery)) {
      return;
    }
    neighbour
      ..busy = true
      ..offered = DateTime.now();
    Socket? socket;
    try {
      socket = await Socket.connect(
        neighbour.address,
        neighbour.port,
        timeout: timeout,
      );
      final frames = _Frames(socket, timeout);
      final hello = await frames.next(_Kind.hello);
      if (hello.length != _idLength) {
        return;
      }
      final parcels = (await host.offer(hello))
          .where((p) => !neighbour.taken.contains(_hex(p.id)))
          .toList();
      final tags = <String, Uint8List>{
        for (final parcel in parcels) _hex(parcel.tag): parcel.tag,
      };
      if (tags.isEmpty) {
        frames.send(_Kind.done, Uint8List(0));
        return;
      }
      final ask = BytesBuilder();
      tags.values.take(_maxTags).forEach(ask.add);
      frames.send(_Kind.ask, ask.toBytes());
      final have = await frames.next(_Kind.have);
      final hosted = {
        for (var at = 0; at + _tagLength <= have.length; at += _tagLength)
          _hex(have.sublist(at, at + _tagLength)),
      };
      final sending = parcels
          .where((p) => hosted.contains(_hex(p.tag)))
          .take(_maxEnvelopes)
          .toList();
      for (final parcel in sending) {
        frames.send(_Kind.envelope, parcel.envelope);
      }
      frames.send(_Kind.done, Uint8List(0));
      for (var i = 0; i < sending.length; i++) {
        final id = await frames.next(_Kind.taken);
        neighbour.taken.add(_hex(id));
      }
    } on Object {
      // The neighbour went away or misbehaved; its next beacon tries again.
    } finally {
      neighbour.busy = false;
      await socket?.close();
    }
  }

  /// Answers a neighbour that has envelopes to hand over.
  Future<void> _serve(Socket socket) async {
    try {
      final frames = _Frames(socket, timeout);
      final nonce = _bytes(_idLength);
      frames.send(_Kind.hello, nonce);
      final first = await frames.any();
      if (first.kind == _Kind.done) {
        return;
      }
      if (first.kind != _Kind.ask || first.data.length % _tagLength != 0) {
        return;
      }
      final mine = host.ownTag(nonce);
      final asked = first.data;
      var hosts = false;
      for (var at = 0; at < asked.length; at += _tagLength) {
        hosts |= _same(asked.sublist(at, at + _tagLength), mine);
      }
      frames.send(_Kind.have, hosts ? mine : Uint8List(0));
      for (var received = 0; received <= _maxEnvelopes; received++) {
        final frame = await frames.any();
        if (frame.kind == _Kind.done) {
          break;
        }
        if (frame.kind != _Kind.envelope || frame.data.length < 3 + 16) {
          return;
        }
        await host.receive(frame.data);
        frames.send(_Kind.taken, frame.data.sublist(3, 3 + _idLength));
      }
      await socket.flush();
    } on Object {
      // A broken exchange: nothing was taken that was not answered.
    } finally {
      await socket.close();
    }
  }

  Uint8List _bytes(int length) =>
      Uint8List.fromList(List.generate(length, (_) => _random.nextInt(256)));
}

class _Neighbour {
  new(this.address, this.port) : heard = DateTime.now();

  InternetAddress address;
  int port;
  DateTime heard;

  /// When it was last offered envelopes; `null` to offer at once.
  DateTime? offered;
  bool busy = false;

  /// Envelope ids it already took: not offered again.
  final taken = <String>{};
}

/// One frame: a kind and its bytes.
class _Frame {
  new(this.kind, this.data);

  final int kind;
  final Uint8List data;
}

/// Length-prefixed frames over a socket: kind (1), length (4), bytes.
class _Frames {
  new(this._socket, this._timeout) {
    _subscription = _socket.listen(
      (chunk) {
        _buffer.add(chunk);
        _wake();
      },
      onError: (Object _) => _close(),
      onDone: _close,
    );
  }

  final Socket _socket;
  final Duration _timeout;
  final _buffer = BytesBuilder(copy: false);
  late final StreamSubscription<Uint8List> _subscription;
  Completer<void>? _waiting;
  bool _closed = false;

  void send(int kind, Uint8List data) {
    final header = ByteData(5)
      ..setUint8(0, kind)
      ..setUint32(1, data.length);
    _socket
      ..add(header.buffer.asUint8List())
      ..add(data);
  }

  /// The next frame, which must be of [kind].
  Future<Uint8List> next(int kind) async {
    final frame = await any();
    if (frame.kind != kind) {
      throw StateError('expected frame $kind, got ${frame.kind}');
    }
    return frame.data;
  }

  /// The next frame, whatever its kind.
  Future<_Frame> any() async {
    while (true) {
      final bytes = _buffer.toBytes();
      if (bytes.length >= 5) {
        final view = ByteData.sublistView(bytes);
        final length = view.getUint32(1);
        if (length > _maxFrame) {
          throw StateError('frame too large');
        }
        if (bytes.length >= 5 + length) {
          _buffer
            ..clear()
            ..add(bytes.sublist(5 + length));
          return _Frame(bytes[0], bytes.sublist(5, 5 + length));
        }
      }
      if (_closed) {
        throw const SocketException('closed');
      }
      final waiting = _waiting = Completer<void>();
      await waiting.future.timeout(_timeout);
    }
  }

  void _wake() {
    final waiting = _waiting;
    _waiting = null;
    if (waiting != null && !waiting.isCompleted) {
      waiting.complete();
    }
  }

  void _close() {
    _closed = true;
    unawaited(_subscription.cancel());
    _wake();
  }
}

bool _same(List<int> a, List<int> b) {
  if (a.length != b.length) {
    return false;
  }
  var difference = 0;
  for (var i = 0; i < a.length; i++) {
    difference |= a[i] ^ b[i];
  }
  return difference == 0;
}

String _hex(List<int> bytes) =>
    bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
