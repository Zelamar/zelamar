import 'package:flutter/material.dart';
import 'package:zelamar/src/design/emblem.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/ui/call/call_screen.dart';
import 'package:zelamar/src/ui/chats/chats_screen.dart';
import 'package:zelamar/src/ui/onboarding/welcome_screen.dart';
import 'package:zelamar/src/ui/words.dart';

/// Root widget of Zelamar: shows whatever the messenger's phase calls for.
///
/// Receives the messenger as a parameter, so the widget tree stays testable
/// without the native library.
class ZelamarApp extends StatelessWidget {
  const new({required this.messenger, super.key});

  final Messenger messenger;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Zelamar',
      debugShowCheckedModeBanner: false,
      theme: zelamarTheme(Brightness.light),
      darkTheme: zelamarTheme(Brightness.dark),
      // A call covers every screen, whichever is open (ADR-0019).
      builder: (context, child) => Stack(
        children: [
          ?child,
          Positioned.fill(child: CallOverlay(messenger: messenger)),
        ],
      ),
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
            ),
            MessengerPhase.ready => ChatsScreen(
              key: const Key('app.chats'),
              messenger: messenger,
            ),
            MessengerPhase.failed => _Failure(
              messenger.failure?.code ?? '',
              key: const Key('app.failed'),
            ),
            MessengerPhase.unlinked => _Unlinked(
              messenger,
              key: const Key('app.unlinked'),
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

/// This device was removed from its account: it can only be erased.
class _Unlinked extends StatefulWidget {
  const new(this.messenger, {super.key});

  final Messenger messenger;

  @override
  State<_Unlinked> createState() => _UnlinkedState();
}

class _UnlinkedState extends State<_Unlinked> {
  bool _busy = false;

  Future<void> _erase() async {
    setState(() => _busy = true);
    try {
      await widget.messenger.eraseThisDevice();
    } on MessengerFailure {
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final text = context.text;
    final name = widget.messenger.username;
    return Scaffold(
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(ZSpace.gutter),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                const SunsetZEmblem(size: 72),
                const SizedBox(height: ZSpace.xl),
                Text(
                  name == null
                      ? 'This device was removed from your account.'
                      : 'This device was removed from @$name.',
                  style: text.titleMedium,
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: ZSpace.xl),
                FilledButton(
                  key: const Key('unlinked.erase'),
                  onPressed: _busy ? null : _erase,
                  child: const Text('Erase this device'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
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
