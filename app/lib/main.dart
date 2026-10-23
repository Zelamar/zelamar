import 'dart:async';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:path_provider/path_provider.dart';
import 'package:zelamar/src/app.dart';
import 'package:zelamar/src/messenger/config.dart';
import 'package:zelamar/src/messenger/key_vault.dart';
import 'package:zelamar/src/messenger/leftovers.dart';
import 'package:zelamar/src/messenger/live_messenger.dart';
import 'package:zelamar/src/messenger/notifier.dart';
import 'package:zelamar/src/messenger/transport.dart';
import 'package:zelamar/src/rust/frb_generated.dart';

Future<void> main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  unawaited(sweepLeftovers());

  final config = MessengerConfig.from(arguments);
  final support = await getApplicationSupportDirectory();
  final separator = Platform.pathSeparator;
  final directory =
      '${support.path}${separator}profiles$separator${config.profile}';
  final messenger = LiveMessenger(
    directory: directory,
    vault: KeyVault(directory, profile: config.profile),
    transport: ServiceTransport(config.service),
    notifier: Notifier.forPlatform(),
  );

  runApp(ZelamarApp(messenger: messenger));
  await messenger.start();
}
