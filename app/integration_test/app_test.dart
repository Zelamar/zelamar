import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/rust/api/diagnostics.dart' as core;
import 'package:zelamar/src/rust/frb_generated.dart';

/// End-to-end smoke test: real Flutter UI + real Rust core on a real device.
void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  setUpAll(RustLib.init);

  testWidgets('UI renders values produced by the Rust core', (tester) async {
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

    // Screenshot capture is implemented by the Android embedding; CI attaches
    // it to the run so the CEO can review the Android build without a phone.
    if (Platform.isAndroid) {
      await binding.convertFlutterSurfaceToImage();
      await tester.pumpAndSettle();
      await binding.takeScreenshot('android-hello');
    }
  });
}
