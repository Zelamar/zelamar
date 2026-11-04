import 'dart:async';
import 'dart:io';

/// Reads [socket] until it closes, or until it stops answering.
///
/// A connection can die without a word: a computer goes to sleep, a phone
/// moves to another network. The app would then wait for messages that
/// never come. So every [every] it asks the mailbox `ping`, which answers
/// `pong` (`protocol/delivery.md`); a connection that has said nothing for
/// twice that long is closed, and the caller makes a new one.
///
/// Each binary frame goes to [onData], in order.
Future<void> readWhileAlive(
  WebSocket socket, {
  required Duration every,
  required void Function(List<int> data) onData,
}) async {
  final ended = Completer<void>();
  void end() {
    if (!ended.isCompleted) {
      ended.complete();
    }
  }

  var heard = DateTime.now();
  final subscription = socket.listen(
    (data) {
      heard = DateTime.now();
      if (data is List<int>) {
        onData(data);
      }
    },
    onError: (Object _) => end(),
    onDone: end,
    cancelOnError: true,
  );
  final beat = Timer.periodic(every, (_) {
    if (DateTime.now().difference(heard) > every * 2) {
      end();
      return;
    }
    try {
      socket.add('ping');
    } on Object {
      end();
    }
  });
  try {
    await ended.future;
  } finally {
    beat.cancel();
    await subscription.cancel();
    unawaited(socket.close().catchError((_) {}));
  }
}
