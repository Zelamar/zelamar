import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart';

void main() {
  const coreInfo = CoreInfo(version: '9.8.7', protocolVersion: 3);

  Future<void> pumpApp(WidgetTester tester) => tester.pumpWidget(
    const ZelamarApp(coreInfo: coreInfo, greeting: 'Hello, Ada'),
  );

  testWidgets('shows the wordmark, the core greeting and core versions', (
    tester,
  ) async {
    await pumpApp(tester);

    expect(find.text('Zelamar'), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('hello.greeting'))).data,
      'Hello, Ada',
    );
    expect(
      tester.widget<Text>(find.byKey(const Key('hello.coreInfo'))).data,
      'Core 9.8.7 · Protocol 3',
    );
  });

  testWidgets('follows the platform dark mode', (tester) async {
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
    addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);

    await pumpApp(tester);

    final context = tester.element(find.byKey(const Key('hello.greeting')));
    expect(Theme.of(context).brightness, Brightness.dark);
  });
}
