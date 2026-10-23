import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'services.dart';
import 'ui/home.dart';
import 'ui/onboarding_screen.dart';
import 'ui/theme.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  LicenseRegistry.addLicense(() async* {
    yield LicenseEntryWithLineBreaks([
      'Geist',
    ], await rootBundle.loadString('assets/fonts/OFL.txt'));
  });
  final services = await Services.start();
  runApp(PickScribeApp(services: services));
}

class PickScribeApp extends StatefulWidget {
  const PickScribeApp({super.key, required this.services});

  final Services services;

  @override
  State<PickScribeApp> createState() => _PickScribeAppState();
}

class _PickScribeAppState extends State<PickScribeApp> with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    widget.services.system.refresh();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  // Permissions and the bubble change in other apps and in the bubble itself, so read them
  // again whenever the screen comes back.
  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) widget.services.system.refresh();
  }

  @override
  Widget build(BuildContext context) {
    return ServicesScope(
      services: widget.services,
      child: MaterialApp(
        title: 'PickScribe',
        debugShowCheckedModeBanner: false,
        theme: buildTheme(),
        home: ValueListenableBuilder(
          valueListenable: widget.services.onboarded,
          builder: (context, onboarded, _) =>
              onboarded ? const HomeShell() : const OnboardingScreen(first: true),
        ),
      ),
    );
  }
}
