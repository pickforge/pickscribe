import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/dictionary.dart';
import 'package:pickscribe/settings.dart';
import 'package:pickscribe/storage.dart';

void main() {
  late Directory dir;

  setUp(() async => dir = await Directory.systemTemp.createTemp('storage'));
  tearDown(() => dir.delete(recursive: true));

  test('overlapping writes land in order and leave no temporary file', () async {
    final file = SafeFile(File('${dir.path}/notes.txt'));
    await Future.wait([for (var i = 0; i < 50; i++) file.write('version $i ' * 2000)]);
    expect(await file.file.readAsString(), 'version 49 ' * 2000);
    expect(await File('${dir.path}/notes.txt.tmp').exists(), isFalse);
  });

  test('a failed write does not stop the next one', () async {
    final file = SafeFile(File('${dir.path}/missing/notes.txt'));
    await expectLater(file.write('lost'), throwsA(isA<FileSystemException>()));
    await Directory('${dir.path}/missing').create();
    await file.write('kept');
    expect(await file.file.readAsString(), 'kept');
  });

  test('a removed dictionary entry stays removed after quick edits', () async {
    final store = DictionaryStore(dir);
    await store.load();
    final entries = [for (var i = 0; i < 30; i++) DictionaryEntry('Term$i')];
    await Future.wait(entries.map(store.add));
    await Future.wait([store.remove(entries.first), store.add(DictionaryEntry('Last'))]);
    final again = DictionaryStore(dir);
    await again.load();
    expect(again.dictionary.terms, isNot(contains('Term0')));
    expect(again.dictionary.terms, contains('Last'));
  });

  test('quick settings changes leave valid settings behind', () async {
    final store = SettingsStore(dir);
    await store.load();
    await Future.wait([
      for (var threads = 1; threads <= 8; threads++)
        store.update(store.settings.copyWith(threads: threads)),
    ]);
    final again = SettingsStore(dir);
    await again.load();
    expect(again.settings.threads, 8);
  });
}
