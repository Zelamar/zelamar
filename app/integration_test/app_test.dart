import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/transport.dart';
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

  testWidgets('the native notifier is wired in this build', (tester) async {
    // A real channel to the runner (C++ on Windows, Kotlin on Android): a
    // missing implementation would throw MissingPluginException here.
    const channel = MethodChannel('zelamar/notify');
    final inFront = await channel.invokeMethod<bool>('inFront');
    expect(inFront, isA<bool>());
  });

  testWidgets('welcome, onboarding, then an encrypted exchange in the lab', (
    tester,
  ) async {
    if (Platform.isAndroid) await binding.convertFlutterSurfaceToImage();

    // The real messenger on a fresh directory: no account yet. No service
    // is reachable in CI, and none is needed to get this far.
    final directory = await Directory.systemTemp.createTemp('zelamar-it-');
    addTearDown(() => directory.delete(recursive: true));
    final messenger = LiveMessenger(
      directory: directory.path,
      vault: KeyVault(directory.path, profile: 'integration'),
      transport: ServiceTransport(Uri.parse('http://127.0.0.1:9')),
      notifier: const SilentNotifier(),
    );
    await tester.pumpWidget(ZelamarApp(messenger: messenger));
    await messenger.start();
    await pumpUntil(tester, find.text('Private by design.'));
    await screenshot(tester, 'welcome');

    await tester.tap(find.byKey(const Key('welcome.create')));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('account.username')), 'ada');
    await tester.pumpAndSettle();
    await screenshot(tester, 'choose-name');
    await tester.pageBack();
    await tester.pumpAndSettle();

    // Open the lab: two real devices start on real encrypted stores.
    await tester.tap(find.byKey(const Key('welcome.lab')));
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
