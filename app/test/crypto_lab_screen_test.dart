import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/lab/crypto_lab_screen.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/rust/api/lab.dart';

/// In-memory stand-in for the Rust lab: echoes what is sent, as a perfect
/// transport would.
class FakeLabEngine implements LabEngine {
  int restarts = 0;
  bool verify = true;

  @override
  List<LabParticipant> get participants => const [
    LabParticipant(
      side: LabSide.alice,
      name: 'Alice',
      fingerprint: 'a1b2 c3d4 e5f6 0718',
    ),
    LabParticipant(
      side: LabSide.bob,
      name: 'Bob',
      fingerprint: '9f8e 7d6c 5b4a 3928',
    ),
  ];

  @override
  Future<LabExchange> send(LabSide from, String text) async => LabExchange(
    from: from,
    text: text,
    envelopeBytes: 1184,
    envelopePreview: '5a450042 13c0ffee …',
    ciphersuite: 'MLS · ML-KEM-768 + X25519 · AES-256-GCM',
    deliveredText: text,
    authorVerified: verify,
    micros: BigInt.from(2300),
  );

  @override
  Future<void> restart() async => restarts++;
}

Future<void> pumpLab(
  WidgetTester tester, {
  required Size size,
  required LabEngineFactory engine,
}) async {
  tester.view
    ..physicalSize = size
    ..devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MaterialApp(
      theme: zelamarTheme(Brightness.light),
      home: CryptoLabScreen(startEngine: engine),
    ),
  );
  await tester.pumpAndSettle();
}

void main() {
  const wide = Size(1400, 900);
  const phone = Size(390, 844);

  testWidgets('wide windows show Alice, the wire and Bob side by side', (
    tester,
  ) async {
    final engine = FakeLabEngine();
    await pumpLab(tester, size: wide, engine: () async => engine);

    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('Bob'), findsOneWidget);
    expect(find.text('ON THE WIRE'), findsOneWidget);
    expect(find.text('a1b2 c3d4 e5f6 0718'), findsOneWidget);
    expect(find.byType(TabBar), findsNothing);
  });

  testWidgets('a message appears for both sides and on the wire', (
    tester,
  ) async {
    await pumpLab(tester, size: wide, engine: () async => FakeLabEngine());

    await tester.enterText(
      find.descendant(
        of: find.byKey(const Key('lab.composer.alice')),
        matching: find.byType(TextField),
      ),
      'Hello, Bob',
    );
    await tester.tap(find.byKey(const Key('lab.send.alice')));
    await tester.pumpAndSettle();

    // Alice's outgoing bubble and Bob's incoming one.
    expect(find.text('Hello, Bob'), findsNWidgets(2));
    expect(find.text('Sender verified'), findsNWidgets(2));
    expect(find.text('Alice → Bob'), findsOneWidget);
    expect(find.text('1184 bytes'), findsOneWidget);
    expect(find.text('2.3 ms'), findsOneWidget);
  });

  testWidgets('an unverified sender is flagged, never hidden', (tester) async {
    final engine = FakeLabEngine()..verify = false;
    await pumpLab(tester, size: wide, engine: () async => engine);

    await tester.enterText(
      find.descendant(
        of: find.byKey(const Key('lab.composer.bob')),
        matching: find.byType(TextField),
      ),
      'trust me',
    );
    await tester.tap(find.byKey(const Key('lab.send.bob')));
    await tester.pumpAndSettle();

    expect(find.text('Sender NOT verified'), findsOneWidget);
    expect(find.text('Not verified'), findsOneWidget);
  });

  testWidgets('restart reopens both devices and says so', (tester) async {
    final engine = FakeLabEngine();
    await pumpLab(tester, size: wide, engine: () async => engine);

    await tester.tap(find.byKey(const Key('lab.restart')));
    await tester.pumpAndSettle();

    expect(engine.restarts, 1);
    expect(
      find.text('Both devices reopened from encrypted storage'),
      findsOneWidget,
    );
    expect(
      find.text('Both devices restarted · state reloaded from encrypted disk'),
      findsOneWidget,
    );
  });

  testWidgets('phones get tabs for Alice, the wire and Bob', (tester) async {
    await pumpLab(tester, size: phone, engine: () async => FakeLabEngine());

    expect(find.byType(TabBar), findsOneWidget);
    expect(find.byKey(const Key('lab.tab.alice')), findsOneWidget);
    expect(find.byKey(const Key('lab.tab.wire')), findsOneWidget);
    expect(find.byKey(const Key('lab.tab.bob')), findsOneWidget);
  });

  testWidgets('a failing start is reported, not swallowed', (tester) async {
    await pumpLab(
      tester,
      size: wide,
      engine: () async => throw const LabError(message: 'disk full'),
    );

    expect(find.textContaining('could not start'), findsOneWidget);
    expect(find.textContaining('disk full'), findsOneWidget);
  });
}
