import 'package:flutter/material.dart';
import 'data/bootstrap_stub.dart'
    if (dart.library.io) 'data/bootstrap_native.dart';
import 'data/settings_store.dart';
import 'data/google_native.dart';
import 'model/google_connection.dart';
import 'model/profile_discovery.dart';
import 'data/profile_discovery_native.dart';
import 'data/native_repository.dart';
import 'model/workspace.dart';
import 'ui/app.dart';
import 'ui/theme.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const Startup());
}

class Startup extends StatefulWidget {
  const Startup({super.key});
  @override
  State<Startup> createState() => _StartupState();
}

class _StartupState extends State<Startup> {
  Workspace? workspace;
  bool failed = false;
  @override
  void initState() {
    super.initState();
    open();
  }

  Future<void> open() async {
    setState(() => failed = false);
    try {
      final repository = await openRepository();
      final google = GoogleConnection(
        NativeGoogleAuthorization(),
        DeviceGoogleConnectionStore(),
      );
      final next = Workspace(
        repository,
        DeviceSettings(),
        google: google,
        profileDiscovery: repository is NativeRepository
            ? ProfileDiscovery(
                google,
                NativeProfileDiscovery(repository),
                namespace: const String.fromEnvironment(
                  'SHEP_PROFILE_NAMESPACE',
                ),
              )
            : null,
      );
      if (!mounted) {
        next.dispose();
        return;
      }
      setState(() => workspace = next);
      await next.initialize();
    } catch (_) {
      if (mounted) setState(() => failed = true);
    }
  }

  @override
  void dispose() {
    workspace?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (workspace case final Workspace ready) return ShepApp(workspace: ready);
    return MaterialApp(
      title: 'Shep',
      debugShowCheckedModeBanner: false,
      theme: shepTheme(Brightness.light),
      darkTheme: shepTheme(Brightness.dark),
      home: Scaffold(
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: failed
                ? Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const Text(
                        'Shep could not open its device cache. Free some storage or unlock the device, then retry.',
                      ),
                      const SizedBox(height: 16),
                      FilledButton(
                        onPressed: open,
                        child: const Text('Retry opening Shep'),
                      ),
                    ],
                  )
                : const CircularProgressIndicator(),
          ),
        ),
      ),
    );
  }
}
