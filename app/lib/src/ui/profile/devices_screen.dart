import 'dart:async';

import 'package:flutter/material.dart';
import 'package:zelamar/src/design/theme.dart';
import 'package:zelamar/src/design/tokens.dart';
import 'package:zelamar/src/messenger/messenger.dart';
import 'package:zelamar/src/rust/api/session.dart';
import 'package:zelamar/src/ui/profile/link_device_screen.dart';
import 'package:zelamar/src/ui/words.dart';

/// The account's devices: link one, remove a lost one, or unlink this one
/// (ADR-0014).
class DevicesScreen extends StatefulWidget {
  const new({required this.messenger, this.recovered = false, super.key});

  final Messenger messenger;

  /// Shown right after a recovery: the lost device should go first
  /// (ADR-0018).
  final bool recovered;

  @override
  State<DevicesScreen> createState() => _DevicesScreenState();
}

class _DevicesScreenState extends State<DevicesScreen> {
  List<DeviceView>? _devices;
  String? _error;
  bool _busy = false;

  Messenger get _messenger => widget.messenger;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final devices = await _messenger.devices();
      if (mounted) {
        setState(() {
          _devices = devices;
          _error = null;
        });
      }
    } on MessengerFailure catch (failure) {
      if (mounted) {
        setState(() => _error = describeFailure(failure.code));
      }
    }
  }

  Future<void> _link() async {
    await Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => LinkDeviceScreen(messenger: _messenger),
      ),
    );
    await _load();
  }

  Future<void> _remove(DeviceView device) async {
    final name = _name(device);
    final sure = await _confirm(
      title: 'Remove “$name”?',
      body:
          'It leaves all your chats and gets no new messages. Do this if it '
          'is lost or no longer yours.',
      action: 'Remove',
    );
    if (sure) {
      await _run(() async {
        await _messenger.removeDevice(device.id);
        await _load();
      });
    }
  }

  Future<void> _unlink() async {
    final sure = await _confirm(
      title: 'Unlink this device?',
      body:
          'Your chats and messages are erased from it. Your other device '
          'keeps everything.',
      action: 'Unlink',
    );
    if (sure) {
      await _run(() async {
        await _messenger.unlinkThisDevice();
        if (mounted) {
          Navigator.of(context).popUntil((route) => route.isFirst);
        }
      });
    }
  }

  Future<void> _run(Future<void> Function() action) async {
    setState(() => _busy = true);
    try {
      await action();
    } on MessengerFailure catch (failure) {
      if (mounted) {
        setState(() => _error = describeFailure(failure.code));
      }
    } finally {
      if (mounted) {
        setState(() => _busy = false);
      }
    }
  }

  Future<bool> _confirm({
    required String title,
    required String body,
    required String action,
  }) async {
    final answer = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(title),
        content: Text(body),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(context).pop(false),
            child: const Text('Cancel'),
          ),
          TextButton(
            key: const Key('devices.confirm'),
            onPressed: () => Navigator.of(context).pop(true),
            style: TextButton.styleFrom(foregroundColor: context.colors.danger),
            child: Text(action),
          ),
        ],
      ),
    );
    return answer ?? false;
  }

  static String _name(DeviceView device) =>
      device.label ?? (device.thisDevice ? 'This device' : 'Linked device');

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final text = context.text;
    final devices = _devices;
    final primary = _messenger.canLinkDevices;
    return Scaffold(
      appBar: AppBar(
        backgroundColor: Colors.transparent,
        surfaceTintColor: Colors.transparent,
      ),
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: ZSpace.gutter),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 440),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    'Devices',
                    style: text.displayMedium?.copyWith(fontSize: 36),
                  ),
                  const SizedBox(height: ZSpace.sm),
                  Text(
                    'Each device has its own keys and sees new messages in '
                    'all your chats.',
                    style: text.bodyMedium,
                  ),
                  if (widget.recovered) ...[
                    const SizedBox(height: ZSpace.xl),
                    Container(
                      key: const Key('devices.recovered'),
                      padding: const EdgeInsets.all(ZSpace.lg),
                      decoration: BoxDecoration(
                        color: colors.surface,
                        borderRadius: BorderRadius.circular(ZRadius.field),
                        border: Border.all(color: colors.hairline),
                      ),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text('Welcome back', style: text.titleMedium),
                          const SizedBox(height: ZSpace.xs),
                          Text(
                            'Remove the devices you no longer have. Until '
                            'then they still get your messages, and your '
                            'chats can’t come back here.',
                            style: text.bodyMedium,
                          ),
                        ],
                      ),
                    ),
                  ],
                  const SizedBox(height: ZSpace.xl),
                  if (devices == null && _error == null)
                    const Center(child: CircularProgressIndicator()),
                  if (devices != null)
                    for (final device in devices)
                      ListTile(
                        key: Key('devices.${device.id}'),
                        contentPadding: EdgeInsets.zero,
                        leading: Icon(
                          _phone(device)
                              ? Icons.smartphone_outlined
                              : Icons.laptop_outlined,
                          color: colors.accent,
                        ),
                        title: Text(_name(device)),
                        subtitle: device.thisDevice
                            ? const Text('This device')
                            : null,
                        trailing: primary && !device.thisDevice
                            ? TextButton(
                                key: Key('devices.remove.${device.id}'),
                                onPressed: _busy ? null : () => _remove(device),
                                style: TextButton.styleFrom(
                                  foregroundColor: colors.danger,
                                ),
                                child: const Text('Remove'),
                              )
                            : null,
                      ),
                  if (_error != null)
                    Padding(
                      padding: const EdgeInsets.only(top: ZSpace.lg),
                      child: Text(
                        _error!,
                        key: const Key('devices.error'),
                        style: text.bodyMedium?.copyWith(color: colors.danger),
                      ),
                    ),
                  const SizedBox(height: ZSpace.xl),
                  if (primary)
                    FilledButton(
                      key: const Key('devices.link'),
                      onPressed: _busy ? null : _link,
                      child: const Text('Link a device'),
                    )
                  else
                    OutlinedButton(
                      key: const Key('devices.unlink'),
                      onPressed: _busy ? null : _unlink,
                      style: OutlinedButton.styleFrom(
                        foregroundColor: colors.danger,
                      ),
                      child: const Text('Unlink this device'),
                    ),
                  const SizedBox(height: ZSpace.xl),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  static bool _phone(DeviceView device) {
    final label = (device.label ?? '').toLowerCase();
    return label.contains('phone') || label.contains('android');
  }
}
