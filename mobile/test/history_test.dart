import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/history.dart';

HistoryItem _item(String id, int at, {String text = 'hello'}) => HistoryItem(
  id: id,
  at: DateTime.fromMillisecondsSinceEpoch(at * 1000),
  text: text,
  transcript: text,
  audioMs: 1500,
  app: 'com.whatsapp',
);

void main() {
  late Directory dir;

  setUp(() async => dir = await Directory.systemTemp.createTemp('history'));
  tearDown(() => dir.delete(recursive: true));

  Future<HistoryStore> reload() async {
    final store = HistoryStore(dir);
    await store.load();
    return store;
  }

  test('keeps the newest first and survives a reload', () async {
    final store = await reload();
    await store.add(_item('1', 100));
    await store.add(_item('2', 200));
    expect((await reload()).items.map((item) => item.id), ['2', '1']);
  });

  test('a line cut short by a crash does not spoil the next dictation', () async {
    final store = await reload();
    await store.add(_item('1', 100));
    final file = File('${dir.path}/history.jsonl');
    await file.writeAsString('{"id": "2", "at', mode: FileMode.append);
    final again = await reload();
    await again.add(_item('3', 300));
    expect((await reload()).items.map((item) => item.id), ['3', '1']);
  });

  test('marks delivery, removes, restores, and clears', () async {
    final store = await reload();
    await store.add(_item('1', 100));
    await store.add(_item('2', 200));
    await store.add(_item('3', 300));
    await store.markDelivered('2', false);
    expect((await reload()).items[1].delivered, isFalse);

    final removed = store.items[1];
    await store.remove('2');
    expect(store.items.map((item) => item.id), ['3', '1']);
    await store.restore(removed);
    expect((await reload()).items.map((item) => item.id), ['3', '2', '1']);

    await store.clear();
    expect((await reload()).items, isEmpty);
  });

  test('drops the oldest past the limit', () async {
    final store = HistoryStore(dir, limit: 2);
    await store.load();
    for (var i = 1; i <= 3; i++) {
      await store.add(_item('$i', i * 100));
    }
    expect((await reload()).items.map((item) => item.id), ['3', '2']);
  });

  test('skips lines that do not parse', () async {
    await File('${dir.path}/history.jsonl')
        .writeAsString('{"id":"1","at":100,"text":"ok"}\nnot json\n{"id":2}\n');
    final store = await reload();
    expect(store.items.single.text, 'ok');
    expect(store.items.single.transcript, 'ok');
  });
}
