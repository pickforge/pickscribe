import 'package:flutter/material.dart';

import 'bench_screen.dart';
import 'pipeline.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  Pipeline().listen();
  runApp(const PickScribeApp());
}

class PickScribeApp extends StatelessWidget {
  const PickScribeApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'PickScribe',
      theme: ThemeData(colorSchemeSeed: const Color(0xFFFF7A1A), useMaterial3: true),
      darkTheme: ThemeData(
        colorSchemeSeed: const Color(0xFFFF7A1A),
        brightness: Brightness.dark,
        useMaterial3: true,
      ),
      home: const BenchScreen(),
    );
  }
}
