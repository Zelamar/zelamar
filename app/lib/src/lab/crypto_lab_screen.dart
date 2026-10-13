import 'dart:async';

import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/design/typography.dart';
import 'package:zelamar/src/lab/lab_engine.dart';
import 'package:zelamar/src/rust/api/lab.dart';

/// Crypto Lab: Alice and Bob, two real devices on the Zelamar core, with the
/// encrypted traffic between them laid bare in the middle.
///
/// Wide windows show three columns; phones show three tabs.
class CryptoLabScreen extends StatefulWidget {
  const new({required this.startEngine, super.key});

  final LabEngineFactory startEngine;

  @override
  State<CryptoLabScreen> createState() => _CryptoLabScreenState();
}

/// One line of the lab's history.
sealed class LabEntry;

/// A message that went from one side to the other.
final class ExchangeEntry extends LabEntry {
  new(this.exchange);

  final LabExchange exchange;
}

/// Both devices were reopened from disk.
final class RestartEntry extends LabEntry;

class _CryptoLabScreenState extends State<CryptoLabScreen> {
  LabEngine? _engine;
  Object? _startError;
  bool _busy = false;
  final List<LabEntry> _entries = [];

  /// Three columns from this width up; tabs below it.
  static const double _wideLayout = 900;

  @override
  void initState() {
    super.initState();
    unawaited(_start());
  }

  Future<void> _start() async {
    try {
      final engine = await widget.startEngine();
      if (mounted) setState(() => _engine = engine);
    } on Object catch (error) {
      if (mounted) setState(() => _startError = error);
    }
  }

  Future<void> _send(LabSide side, String text) async {
    final engine = _engine;
    final message = text.trim();
    if (engine == null || message.isEmpty || _busy) return;
    setState(() => _busy = true);
    try {
      final exchange = await engine.send(side, message);
      if (mounted) setState(() => _entries.add(ExchangeEntry(exchange)));
    } on LabError catch (error) {
      _toast(error.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _restart() async {
    final engine = _engine;
    if (engine == null || _busy) return;
    setState(() => _busy = true);
    try {
      await engine.restart();
      if (!mounted) return;
      setState(() => _entries.add(RestartEntry()));
      _toast('Both devices reopened from encrypted storage');
    } on LabError catch (error) {
      _toast(error.message);
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  static String _describe(Object error) =>
      error is LabError ? error.message : '$error';

  void _toast(String message) {
    if (!mounted) return;
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(message)));
  }

  @override
  Widget build(BuildContext context) {
    final engine = _engine;
    return Scaffold(
      body: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _Header(onRestart: engine == null || _busy ? null : _restart),
            Expanded(
              child: switch ((engine, _startError)) {
                (_, final Object error) => _Message(
                  'The lab could not start: ${_describe(error)}',
                ),
                (null, _) => const Center(child: CircularProgressIndicator()),
                (final LabEngine engine, _) => LayoutBuilder(
                  builder: (context, constraints) {
                    final alice = _PersonPanel(
                      person: engine.participants[0],
                      entries: _entries,
                      busy: _busy,
                      onSend: _send,
                    );
                    final bob = _PersonPanel(
                      person: engine.participants[1],
                      entries: _entries,
                      busy: _busy,
                      onSend: _send,
                    );
                    final wire = _WirePanel(entries: _entries);
                    return constraints.maxWidth >= _wideLayout
                        ? _WideLayout(alice: alice, wire: wire, bob: bob)
                        : _TabbedLayout(alice: alice, wire: wire, bob: bob);
                  },
                ),
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const new({required this.onRestart});

  final VoidCallback? onRestart;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final title = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text('Crypto Lab', style: text.headlineSmall?.copyWith(fontSize: 28)),
        const SizedBox(height: ZSpace.xxs),
        Text(
          'Two devices, one real Zelamar core. Nothing leaves this device.',
          style: text.bodySmall,
        ),
      ],
    );
    final restart = OutlinedButton.icon(
      key: const Key('lab.restart'),
      onPressed: onRestart,
      icon: const Icon(Icons.restart_alt_rounded, size: 18),
      label: const Text('Restart devices'),
    );
    final cipher = Tooltip(
      message: 'Hybrid post-quantum key exchange',
      child: Container(
        padding: const EdgeInsets.symmetric(
          horizontal: ZSpace.md,
          vertical: ZSpace.sm,
        ),
        decoration: BoxDecoration(
          color: colors.surfaceSunken,
          borderRadius: BorderRadius.circular(ZRadius.field),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.lock_rounded, size: 14, color: colors.accent),
            const SizedBox(width: ZSpace.xs),
            Text('ML-KEM-768 + X25519', style: text.bodySmall),
          ],
        ),
      ),
    );

    return Padding(
      padding: const EdgeInsets.fromLTRB(
        ZSpace.sm,
        ZSpace.md,
        ZSpace.gutter,
        ZSpace.md,
      ),
      child: LayoutBuilder(
        builder: (context, constraints) {
          // Below this width the actions move under the title.
          if (constraints.maxWidth < 720) {
            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    const BackButton(),
                    const SizedBox(width: ZSpace.xs),
                    Expanded(child: title),
                  ],
                ),
                const SizedBox(height: ZSpace.sm),
                Padding(
                  padding: const EdgeInsets.only(left: ZSpace.sm),
                  child: Wrap(
                    spacing: ZSpace.sm,
                    runSpacing: ZSpace.sm,
                    crossAxisAlignment: WrapCrossAlignment.center,
                    children: [restart, cipher],
                  ),
                ),
              ],
            );
          }
          return Row(
            children: [
              const BackButton(),
              const SizedBox(width: ZSpace.xs),
              Expanded(child: title),
              const SizedBox(width: ZSpace.md),
              restart,
              const SizedBox(width: ZSpace.sm),
              cipher,
            ],
          );
        },
      ),
    );
  }
}

class _WideLayout extends StatelessWidget {
  const new({required this.alice, required this.wire, required this.bob});

  final Widget alice;
  final Widget wire;
  final Widget bob;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    Widget card(Widget child) => DecoratedBox(
      decoration: BoxDecoration(
        color: colors.surfaceRaised,
        border: Border.all(color: colors.hairline),
        borderRadius: BorderRadius.circular(ZRadius.card),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(ZRadius.card),
        child: child,
      ),
    );
    return Padding(
      padding: const EdgeInsets.fromLTRB(
        ZSpace.gutter,
        0,
        ZSpace.gutter,
        ZSpace.gutter,
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(flex: 3, child: card(alice)),
          const SizedBox(width: ZSpace.lg),
          Expanded(flex: 4, child: card(wire)),
          const SizedBox(width: ZSpace.lg),
          Expanded(flex: 3, child: card(bob)),
        ],
      ),
    );
  }
}

class _TabbedLayout extends StatelessWidget {
  const new({required this.alice, required this.wire, required this.bob});

  final Widget alice;
  final Widget wire;
  final Widget bob;

  @override
  Widget build(BuildContext context) {
    return DefaultTabController(
      length: 3,
      child: Column(
        children: [
          const TabBar(
            tabs: [
              Tab(key: Key('lab.tab.alice'), text: 'Alice'),
              Tab(key: Key('lab.tab.wire'), text: 'On the wire'),
              Tab(key: Key('lab.tab.bob'), text: 'Bob'),
            ],
          ),
          Expanded(child: TabBarView(children: [alice, wire, bob])),
        ],
      ),
    );
  }
}

/// One participant's view of the conversation, with a composer.
class _PersonPanel extends StatelessWidget {
  const new({
    required this.person,
    required this.entries,
    required this.busy,
    required this.onSend,
  });

  final LabParticipant person;
  final List<LabEntry> entries;
  final bool busy;
  final Future<void> Function(LabSide side, String text) onSend;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final exchanges = [
      for (final entry in entries)
        if (entry is ExchangeEntry) entry.exchange,
    ];
    final id = person.side.name;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.all(ZSpace.lg),
          child: Row(
            children: [
              CircleAvatar(
                radius: 20,
                backgroundColor: colors.surfaceSunken,
                child: Text(
                  person.name.substring(0, 1),
                  style: text.headlineSmall?.copyWith(fontSize: 18),
                ),
              ),
              const SizedBox(width: ZSpace.md),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(person.name, style: text.titleMedium),
                    Text(
                      person.fingerprint,
                      style: zelamarMono(size: 11.5)
                          .copyWith(color: colors.textTertiary),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
        const Divider(),
        Expanded(
          child: exchanges.isEmpty
              ? _Message('No messages yet. Say hello as ${person.name}.')
              : ListView.builder(
                  reverse: true,
                  padding: const EdgeInsets.all(ZSpace.lg),
                  itemCount: exchanges.length,
                  itemBuilder: (context, index) {
                    final exchange = exchanges[exchanges.length - 1 - index];
                    return exchange.from == person.side
                        ? _Bubble.outgoing(exchange.text)
                        : _Bubble.incoming(
                            exchange.deliveredText,
                            verified: exchange.authorVerified,
                          );
                  },
                ),
        ),
        _Composer(
          key: Key('lab.composer.$id'),
          sendKey: Key('lab.send.$id'),
          hint: 'Message as ${person.name}',
          enabled: !busy,
          onSend: (message) => onSend(person.side, message),
        ),
      ],
    );
  }
}

class _Bubble extends StatelessWidget {
  const new outgoing(this.message) : incoming = false, verified = true;

  const new incoming(this.message, {required this.verified}) : incoming = true;

  final String message;
  final bool incoming;
  final bool verified;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    const big = Radius.circular(ZRadius.bubble);
    const tail = Radius.circular(ZRadius.bubbleTail);

    final bubble = Container(
      constraints: const BoxConstraints(maxWidth: 320),
      padding: const EdgeInsets.fromLTRB(
        ZSpace.md + 2,
        ZSpace.sm + 2,
        ZSpace.md + 2,
        ZSpace.sm + 2,
      ),
      decoration: BoxDecoration(
        color: incoming ? colors.bubbleIncoming : colors.bubbleOutgoing,
        border: incoming
            ? Border.all(color: colors.bubbleIncomingBorder)
            : null,
        borderRadius: BorderRadius.only(
          topLeft: big,
          topRight: big,
          bottomLeft: incoming ? tail : big,
          bottomRight: incoming ? big : tail,
        ),
      ),
      child: Text(
        message,
        style: text.bodyLarge?.copyWith(
          color: incoming ? colors.textPrimary : colors.onBubbleOutgoing,
        ),
      ),
    );

    return Padding(
      padding: const EdgeInsets.only(bottom: ZSpace.sm),
      child: Column(
        crossAxisAlignment: incoming
            ? CrossAxisAlignment.start
            : CrossAxisAlignment.end,
        children: [
          bubble,
          if (incoming)
            Padding(
              padding: const EdgeInsets.only(top: ZSpace.xs, left: ZSpace.xs),
              child: _VerifiedLabel(verified: verified),
            ),
        ],
      ),
    );
  }
}

class _VerifiedLabel extends StatelessWidget {
  const new({required this.verified});

  final bool verified;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final color = verified ? colors.online : colors.danger;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(
          verified ? Icons.verified_rounded : Icons.gpp_bad_rounded,
          size: 14,
          color: color,
        ),
        const SizedBox(width: ZSpace.xs),
        Text(
          verified ? 'Sender verified' : 'Sender NOT verified',
          style: context.text.labelSmall?.copyWith(color: color),
        ),
      ],
    );
  }
}

class _Composer extends StatefulWidget {
  const new({
    required this.sendKey,
    required this.hint,
    required this.enabled,
    required this.onSend,
    super.key,
  });

  final Key sendKey;
  final String hint;
  final bool enabled;
  final Future<void> Function(String message) onSend;

  @override
  State<_Composer> createState() => _ComposerState();
}

class _ComposerState extends State<_Composer> {
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final message = _controller.text;
    if (message.trim().isEmpty) return;
    _controller.clear();
    await widget.onSend(message);
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Container(
      padding: const EdgeInsets.all(ZSpace.md),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: colors.hairline)),
      ),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _controller,
              enabled: widget.enabled,
              textInputAction: TextInputAction.send,
              onSubmitted: (_) => _submit(),
              decoration: InputDecoration(hintText: widget.hint),
            ),
          ),
          const SizedBox(width: ZSpace.sm),
          IconButton.filled(
            key: widget.sendKey,
            tooltip: 'Send',
            onPressed: widget.enabled ? _submit : null,
            icon: const Icon(Icons.arrow_upward_rounded),
          ),
        ],
      ),
    );
  }
}

/// The middle column: every envelope as it crossed the wire.
class _WirePanel extends StatelessWidget {
  const new({required this.entries});

  final List<LabEntry> entries;

  @override
  Widget build(BuildContext context) {
    final text = context.text;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(
            ZSpace.lg,
            ZSpace.lg + ZSpace.xs,
            ZSpace.lg,
            ZSpace.md,
          ),
          child: Text('ON THE WIRE', style: text.labelMedium),
        ),
        const Divider(),
        Expanded(
          child: entries.isEmpty
              ? const _Message(
                  'Send a message to watch it travel: encrypted, padded, '
                  'and readable only by the other side.',
                )
              : ListView.builder(
                  reverse: true,
                  padding: const EdgeInsets.all(ZSpace.lg),
                  itemCount: entries.length,
                  itemBuilder: (context, index) =>
                      switch (entries[entries.length - 1 - index]) {
                        ExchangeEntry(:final exchange) => _WireCard(exchange),
                        RestartEntry() => const _RestartNote(),
                      },
                ),
        ),
      ],
    );
  }
}

class _WireCard extends StatelessWidget {
  const new(this.exchange);

  final LabExchange exchange;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final (from, to) = exchange.from == LabSide.alice
        ? ('Alice', 'Bob')
        : ('Bob', 'Alice');
    final millis = (exchange.micros.toDouble() / 1000).toStringAsFixed(1);

    return Container(
      margin: const EdgeInsets.only(bottom: ZSpace.md),
      padding: const EdgeInsets.all(ZSpace.lg),
      decoration: BoxDecoration(
        color: colors.surface,
        border: Border.all(color: colors.hairline),
        borderRadius: BorderRadius.circular(ZRadius.field + 4),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Text('$from → $to', style: text.titleSmall),
              const Spacer(),
              Text('$millis ms', style: text.labelSmall),
            ],
          ),
          const SizedBox(height: ZSpace.sm),
          SelectableText(
            exchange.envelopePreview,
            style: zelamarMono().copyWith(color: colors.textSecondary),
          ),
          const SizedBox(height: ZSpace.md),
          Wrap(
            spacing: ZSpace.sm,
            runSpacing: ZSpace.sm,
            children: [
              _Chip('${exchange.envelopeBytes} bytes'),
              _Chip(exchange.ciphersuite, icon: Icons.lock_rounded),
              _Chip(
                exchange.authorVerified ? 'Sender verified' : 'Not verified',
                icon: exchange.authorVerified
                    ? Icons.verified_rounded
                    : Icons.gpp_bad_rounded,
                color: exchange.authorVerified ? colors.online : colors.danger,
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  const new(this.label, {this.icon, this.color});

  final String label;
  final IconData? icon;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final tint = color ?? colors.textSecondary;
    return Container(
      padding: const EdgeInsets.symmetric(
        horizontal: ZSpace.sm + 2,
        vertical: ZSpace.xs,
      ),
      decoration: BoxDecoration(
        color: colors.surfaceSunken,
        borderRadius: BorderRadius.circular(ZRadius.field),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null) ...[
            Icon(icon, size: 13, color: tint),
            const SizedBox(width: ZSpace.xs),
          ],
          Flexible(
            child: Text(
              label,
              overflow: TextOverflow.ellipsis,
              style: context.text.labelSmall?.copyWith(color: tint),
            ),
          ),
        ],
      ),
    );
  }
}

class _RestartNote extends StatelessWidget {
  const new();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: ZSpace.md),
      child: Text(
        'Both devices restarted · state reloaded from encrypted disk',
        textAlign: TextAlign.center,
        style: context.text.labelSmall?.copyWith(
          color: context.colors.textTertiary,
        ),
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const new(this.message);

  final String message;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(ZSpace.xl),
        child: Text(
          message,
          textAlign: TextAlign.center,
          style: context.text.bodyMedium,
        ),
      ),
    );
  }
}
