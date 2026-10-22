import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:pickscribe/dictionary.dart';

void main() {
  test('parses terms and corrections and skips comments and bad lines', () {
    final dictionary = Dictionary.parse(
      '# comment\nPickforge\nBitforge -> Pickforge\n  Prova   Clara \n-> nothing\n\n',
    );
    expect(dictionary.entries.map((entry) => entry.line), [
      'Pickforge',
      'Bitforge -> Pickforge',
      'Prova Clara',
    ]);
  });

  test('round-trips through the file format', () {
    final dictionary = Dictionary.parse('Pickforge\nBitforge -> Pickforge\n');
    final again = Dictionary.parse(dictionary.serialize());
    expect(again.entries.map((entry) => entry.line), ['Pickforge', 'Bitforge -> Pickforge']);
  });

  test('builds the glossary prompt from distinct spellings', () {
    final dictionary = Dictionary.parse('Pickforge\nBitforge -> Pickforge\nProva Clara\n');
    expect(dictionary.prompt, 'Glossary: Pickforge, Prova Clara.');
    expect(Dictionary().prompt, '');
  });

  test('corrects whole words only, ignoring case', () {
    final dictionary = Dictionary.parse('Bitforge -> Pickforge\nclod -> Claude\n');
    expect(
      dictionary.apply('O bitforge e o BITFORGE, não bitforges. Clod, clodx.'),
      'O Pickforge e o Pickforge, não bitforges. Claude, clodx.',
    );
  });

  test('treats accented letters as part of a word', () {
    expect(replaceWord('café cafézinho', 'café', 'coffee'), 'coffee cafézinho');
  });

  test('rejects entries the desktop would reject', () {
    expect(() => DictionaryEntry('  '), throwsFormatException);
    expect(() => DictionaryEntry('a -> b'), throwsFormatException);
    expect(() => DictionaryEntry('#tag'), throwsFormatException);
    expect(() => DictionaryEntry('x' * 101), throwsFormatException);
    expect(() => DictionaryEntry('Pickforge', heard: 'pickforge'), throwsFormatException);
    expect(DictionaryEntry('Pickforge', heard: '  ').heard, isNull);
  });

  test('ignores duplicates by case', () {
    final dictionary = Dictionary();
    expect(dictionary.add(DictionaryEntry('Pickforge')), isTrue);
    expect(dictionary.add(DictionaryEntry('pickforge')), isFalse);
    expect(dictionary.remove(DictionaryEntry('PICKFORGE')), isTrue);
    expect(dictionary.entries, isEmpty);
  });

  test('the store saves and loads the file', () async {
    final dir = await Directory.systemTemp.createTemp('dictionary');
    addTearDown(() => dir.delete(recursive: true));
    final store = DictionaryStore(dir);
    await store.load();
    await store.add(DictionaryEntry('Pickforge', heard: 'Bitforge'));
    final again = DictionaryStore(dir);
    await again.load();
    expect(again.dictionary.entries.single.line, 'Bitforge -> Pickforge');
  });
}
