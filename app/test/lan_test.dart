import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/drift/lan.dart';
import 'package:zelamar/src/rust/api/session.dart';

/// A device with one mailbox, named by a number; tags are the mailbox
/// mixed into the nonce, which is all the exchange needs to tell them
/// apart.
class FakeHost implements LanHost {
  new(this.mailbox);

  final int mailbox;

  /// Envelopes waiting, as `(mailbox, id byte)`.
  final waiting = <(int, int)>[];
  final received = <int>[];

  static Uint8List tagOf(int mailbox, Uint8List nonce) =>
      Uint8List.fromList(List.generate(16, (i) => mailbox ^ nonce[i]));

  static Uint8List envelope(int id) =>
      Uint8List.fromList([0x5A, 0x45, 0, ...List.filled(16, id), 1, 2, 3]);

  @override
  Future<List<LanParcel>> offer(Uint8List nonce) async => [
    for (final (mailbox, id) in waiting)
      LanParcel(
        tag: tagOf(mailbox, nonce),
        id: Uint8List.fromList(List.filled(16, id)),
        envelope: envelope(id),
      ),
  ];

  @override
  Uint8List ownTag(Uint8List nonce) => tagOf(mailbox, nonce);

  @override
  Future<void> receive(Uint8List envelope) async => received.add(envelope[3]);
}

void main() {
  final here = InternetAddress.loopbackIPv4;

  Future<(LanNode, FakeHost)> node(int mailbox) async {
    final host = FakeHost(mailbox);
    final node = LanNode(host, retryEvery: Duration.zero);
    await node.start(announce: false);
    addTearDown(node.stop);
    return (node, host);
  }

  test('a neighbour gets only the envelopes for its own mailbox', () async {
    final (alice, alicesHost) = await node(1);
    final (bob, bobsHost) = await node(2);
    alicesHost.waiting.addAll([(2, 10), (3, 11), (2, 12)]);

    await alice.handTo(here, bob.listening!);
    expect(bobsHost.received, [10, 12]);

    // Taken once, not offered again.
    await alice.handTo(here, bob.listening!);
    expect(bobsHost.received, [10, 12]);

    // Something new is.
    alicesHost.waiting.add((2, 13));
    await alice.handTo(here, bob.listening!);
    expect(bobsHost.received, [10, 12, 13]);
  });

  test('nothing to hand over is a short, quiet exchange', () async {
    final (alice, _) = await node(1);
    final (bob, bobsHost) = await node(2);
    await alice.handTo(here, bob.listening!);
    expect(bobsHost.received, isEmpty);
  });

  test('a neighbour hosting none of the mailboxes gets nothing', () async {
    final (alice, alicesHost) = await node(1);
    final (carol, carolsHost) = await node(3);
    alicesHost.waiting.add((2, 10));
    await alice.handTo(here, carol.listening!);
    expect(carolsHost.received, isEmpty);
  });

  test('junk on the line changes nothing', () async {
    final (alice, alicesHost) = await node(1);
    final (bob, bobsHost) = await node(2);
    final junk = await Socket.connect(here, bob.listening!);
    junk.add(List.filled(64, 0xFF));
    await junk.flush();
    await junk.close();

    alicesHost.waiting.add((2, 10));
    await alice.handTo(here, bob.listening!);
    expect(bobsHost.received, [10]);
  });
}
