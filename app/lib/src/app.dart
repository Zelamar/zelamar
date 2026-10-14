import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/chats/chats_screen.dart';
import 'package:zelamar/src/ui/onboarding/welcome_screen.dart';
import 'package:zelamar/src/ui/words.dart';

/// Root widget of Zelamar: shows whatever the messenger's phase calls for.
///
/// Receives the messenger and the lab as parameters, so the widget tree
/// stays testable without the native library.
class ZelamarApp extends StatelessWidget {
  const new({
    required this.messenger,
    this.startLab = RustLabEngine.start,
    super.key,
  });

  final Messenger messenger;
  final LabEngineFactory startLab;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Zelamar',
      debugShowCheckedModeBanner: false,
      theme: zelamarTheme(Brightness.light),
      darkTheme: zelamarTheme(Brightness.dark),
      home: ListenableBuilder(
        listenable: messenger,
        builder: (context, _) => AnimatedSwitcher(
          duration: ZMotion.slow,
          switchInCurve: ZMotion.enter,
          switchOutCurve: ZMotion.exit,
          child: switch (messenger.phase) {
            MessengerPhase.starting => const _Splash(key: Key('app.starting')),
            MessengerPhase.needsAccount => WelcomeScreen(
              key: const Key('app.welcome'),
              messenger: messenger,
              startLab: startLab,
            ),
            MessengerPhase.ready => ChatsScreen(
              key: const Key('app.chats'),
              messenger: messenger,
            ),
            MessengerPhase.failed => _Failure(
              messenger.failure?.code ?? '',
              key: const Key('app.failed'),
            ),
          },
        ),
      ),
    );
  }
}

class _Splash extends StatelessWidget {
  const new({super.key});

  @override
  Widget build(BuildContext context) {
    return const Scaffold(body: Center(child: SunsetZEmblem(size: 96)));
  }
}

class _Failure extends StatelessWidget {
  const new(this.code, {super.key});

  final String code;

  @override
  Widget build(BuildContext context) {
    final text = context.text;
    return Scaffold(
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(ZSpace.gutter),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const SunsetZEmblem(size: 72),
              const SizedBox(height: ZSpace.xl),
              Text(
                'Zelamar can’t open this device’s data.',
                style: text.titleMedium,
                textAlign: TextAlign.center,
              ),
              const SizedBox(height: ZSpace.sm),
              Text(
                describeFailure(code),
                style: text.bodyMedium,
                textAlign: TextAlign.center,
              ),
            ],
          ),
        ),
      ),
    );
  }
}
