import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/main.dart';
import 'package:pickscribe/services.dart';

void fakePlatform({required bool service}) {
  final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  messenger.setMockMethodCallHandler(const MethodChannel('pickscribe/system'), (call) async {
    return switch (call.method) {
      'status' => {'microphone': true, 'service': service, 'version': '0.1.0'},
      'describeApps' => <Object?>[],
      _ => null,
    };
  });
  messenger.setMockMethodCallHandler(const MethodChannel('pickscribe/bubble'), (call) async {
    return {'size': 56, 'opacity': 0.9, 'blocked': <String>[], 'snoozedUntil': 0};
  });
}

Future<Services> start(WidgetTester tester, {bool onboarded = true}) async {
  // A phone-sized screen, so the setup steps fit without scrolling.
  tester.view.physicalSize = const Size(412, 915);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final services = await tester.runAsync(() async {
    final dir = await Directory.systemTemp.createTemp('pickscribe-app');
    addTearDown(() => dir.delete(recursive: true));
    if (onboarded) await File('${dir.path}/app.json').writeAsString('{"onboarded":true}');
    await File('${dir.path}/history.jsonl').writeAsString(
      '{"id":"1","at":${DateTime.now().millisecondsSinceEpoch ~/ 1000},"text":"Olá mundo","audio_ms":1200}\n',
    );
    return Services.start(dir: dir);
  });
  return services!;
}

void main() {
  testWidgets('first launch shows setup, and skipping it opens History', (tester) async {
    fakePlatform(service: false);
    final services = await start(tester, onboarded: false);
    await tester.pumpWidget(PickScribeApp(services: services));
    await tester.pumpAndSettle();
    expect(find.text('Set up PickScribe'), findsOneWidget);

    await tester.tap(find.text('Skip for now'));
    await tester.pumpAndSettle();
    expect(find.text('Olá mundo'), findsOneWidget);
    expect(find.text('Finish setup'), findsOneWidget);
  });

  testWidgets('the tabs open Dictionary and Settings', (tester) async {
    fakePlatform(service: true);
    final services = await start(tester);
    await tester.pumpWidget(PickScribeApp(services: services));
    await tester.pumpAndSettle();
    expect(find.text('Finish setup'), findsOneWidget, reason: 'no model is downloaded');

    await tester.tap(find.text('Dictionary').last);
    await tester.pumpAndSettle();
    expect(find.text('Add word'), findsOneWidget);

    await tester.tap(find.text('Settings').last);
    await tester.pumpAndSettle();
    expect(find.text('Hidden in apps'), findsOneWidget);
    expect(find.text('Portuguese, English'), findsOneWidget);
  });
}
