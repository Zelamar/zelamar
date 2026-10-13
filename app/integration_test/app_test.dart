import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart' as core;
import 'package:zelamar/src/rust/frb_generated.dart';

/// Pumps frames until [finder] matches, for work that completes off the UI
/// thread (calls into the Rust core).
Future<void> pumpUntil(
  WidgetTester tester,
  Finder finder, {
  Duration timeout = const Duration(seconds: 30),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (finder.evaluate().isEmpty) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Timed out waiting for $finder');
    }
    await tester.pump(const Duration(milliseconds: 100));
  }
  await tester.pumpAndSettle();
}

/// End-to-end: real Flutter UI + real Rust core on a real device. Android
/// screenshots are attached to CI runs so the CEO can review the phone build
/// without a phone.
void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(RustLib.init);

  Future<void> screenshot(WidgetTester tester, String name) async {
    if (!Platform.isAndroid) return;
    await tester.pumpAndSettle();
    await binding.takeScreenshot('android-$name');
  }

  testWidgets('hello screen, then an encrypted exchange in the Crypto Lab', (
    tester,
  ) async {
    if (Platform.isAndroid) await binding.convertFlutterSurfaceToImage();

    final info = core.coreInfo();
    await tester.pumpWidget(
      ZelamarApp(
        coreInfo: info,
        greeting: core.greet(name: ''),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Hello, Zelamar'), findsOneWidget);
    expect(
      find.text('Core ${info.version} · Protocol ${info.protocolVersion}'),
      findsOneWidget,
    );
    await screenshot(tester, 'hello');

    // Open the lab: two real devices start on real encrypted stores.
    await tester.tap(find.byKey(const Key('hello.openLab')));
    final aliceComposer = find.descendant(
      of: find.byKey(const Key('lab.composer.alice')),
      matching: find.byType(TextField),
    );
    await pumpUntil(tester, aliceComposer);

    await tester.enterText(aliceComposer, 'Hello, Bob');
    await tester.tap(find.byKey(const Key('lab.send.alice')));
    await pumpUntil(tester, find.text('Hello, Bob'));
    await screenshot(tester, 'lab-alice');

    final tabbed = find.byType(TabBar).evaluate().isNotEmpty;
    if (tabbed) {
      await tester.tap(find.byKey(const Key('lab.tab.wire')));
      await tester.pumpAndSettle();
    }
    expect(find.text('Alice → Bob'), findsOneWidget);
    await screenshot(tester, 'lab-wire');

    if (tabbed) {
      await tester.tap(find.byKey(const Key('lab.tab.bob')));
      await tester.pumpAndSettle();
    }
    // Bob decrypted Alice's message and authenticated her account.
    expect(find.text('Sender verified'), findsWidgets);
    expect(find.text('Hello, Bob'), findsWidgets);
    await screenshot(tester, 'lab-bob');
  });
}
