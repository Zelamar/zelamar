import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/messenger/heartbeat.dart';

void main() {
  const every = Duration(milliseconds: 250);

  /// A mailbox on this machine that answers `ping` when it should
  /// [answer]: the app's end of the connection, and the mailbox.
  Future<(WebSocket, Future<_Mailbox>)> connect({required bool answer}) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    final accepted = Completer<_Mailbox>();
    server.listen((request) async {
      final socket = await WebSocketTransformer.upgrade(request);
      accepted.complete(_Mailbox(socket, answer: answer));
    });
    final client = await WebSocket.connect('ws://127.0.0.1:${server.port}');
    return (client, accepted.future);
  }

  test('a mailbox that answers keeps the connection, envelopes go through, '
      'and its close ends the reading', () async {
    final (client, accepted) = await connect(answer: true);
    final received = <List<int>>[];
    var ended = false;
    final reading = readWhileAlive(
      client,
      every: every,
      onData: received.add,
    ).whenComplete(() => ended = true);

    final mailbox = await accepted;
    mailbox.socket.add([1, 2, 3]);
    await Future<void>.delayed(every * 6);
    expect(ended, isFalse, reason: 'quiet but answering is alive');
    expect(received, [
      [1, 2, 3],
    ]);

    await mailbox.close();
    await reading.timeout(const Duration(seconds: 5));
  });

  test('a connection that went silent is dropped, though nothing said '
      'it ended', () async {
    final (client, accepted) = await connect(answer: false);
    final started = DateTime.now();
    await readWhileAlive(
      client,
      every: every,
      onData: (_) {},
    ).timeout(const Duration(seconds: 5));
    expect(DateTime.now().difference(started), greaterThan(every * 2));

    // And the mailbox sees it go, so it wakes the phone instead.
    await (await accepted).gone.timeout(const Duration(seconds: 5));
  });
}

class _Mailbox {
  new(this.socket, {required bool answer}) {
    socket.listen((data) {
      if (answer && data == 'ping' && !_closing) {
        socket.add('pong');
      }
    }, onDone: _gone.complete);
  }

  final WebSocket socket;
  final _gone = Completer<void>();
  bool _closing = false;

  /// Completes when the app's end went away.
  Future<void> get gone => _gone.future;

  Future<void> close() {
    _closing = true;
    return socket.close();
  }
}
