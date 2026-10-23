import 'package:flutter/material.dart';

import '../dictionary.dart';
import '../services.dart';
import 'theme.dart';
import 'widgets.dart';

class DictionaryScreen extends StatelessWidget {
  const DictionaryScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final store = ServicesScope.of(context).dictionary;
    return Scaffold(
      backgroundColor: Colors.transparent,
      floatingActionButton: FloatingActionButton.extended(
        onPressed: () => showEntrySheet(context),
        icon: const Icon(Icons.add_rounded),
        label: const Text('Add word'),
      ),
      body: ListenableBuilder(
        listenable: store,
        builder: (context, _) {
          final entries = store.dictionary.entries.reversed.toList();
          return CustomScrollView(
            slivers: [
              const SliverToBoxAdapter(child: PageHeader('Dictionary')),
              if (entries.isEmpty)
                const SliverFillRemaining(
                  hasScrollBody: false,
                  child: EmptyState('Add names and terms so Whisper spells them right.'),
                )
              else
                SliverPadding(
                  padding: const EdgeInsets.fromLTRB(16, 4, 16, 96),
                  sliver: SliverToBoxAdapter(
                    child: SurfaceCard(
                      child: Column(
                        children: [
                          for (var i = 0; i < entries.length; i++) ...[
                            if (i > 0) const Divider(indent: 16, endIndent: 16),
                            _EntryRow(entry: entries[i]),
                          ],
                        ],
                      ),
                    ),
                  ),
                ),
            ],
          );
        },
      ),
    );
  }
}

class _EntryRow extends StatelessWidget {
  const _EntryRow({required this.entry});

  final DictionaryEntry entry;

  @override
  Widget build(BuildContext context) {
    final store = ServicesScope.of(context).dictionary;
    return Dismissible(
      key: ValueKey(entry.line),
      direction: DismissDirection.endToStart,
      background: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 24),
        color: Palette.danger.withValues(alpha: 0.16),
        child: const Icon(Icons.delete_outline_rounded, color: Palette.danger),
      ),
      onDismissed: (_) => _remove(context, store),
      child: ListTile(
        minTileHeight: 56,
        title: Text(entry.text, style: const TextStyle(fontSize: 16, fontWeight: FontWeight.w500)),
        subtitle: entry.heard == null
            ? null
            : Text.rich(
                TextSpan(
                  children: [
                    const TextSpan(text: 'Heard as '),
                    TextSpan(
                      text: entry.heard,
                      style: const TextStyle(
                        color: Palette.text,
                        decoration: TextDecoration.lineThrough,
                        decorationColor: Palette.faint,
                      ),
                    ),
                  ],
                ),
              ),
        trailing: IconButton(
          tooltip: 'Remove',
          icon: const Icon(Icons.close_rounded, size: 20, color: Palette.faint),
          onPressed: () => _remove(context, store),
        ),
      ),
    );
  }

  void _remove(BuildContext context, DictionaryStore store) {
    store.remove(entry);
    ScaffoldMessenger.of(context)
      ..hideCurrentSnackBar()
      ..showSnackBar(
        SnackBar(
          content: Text('Removed ${entry.text}'),
          action: SnackBarAction(label: 'Undo', onPressed: () => store.add(entry)),
        ),
      );
  }
}

/// Asks for a spelling and, for a correction, what Whisper hears instead.
Future<void> showEntrySheet(BuildContext context, {String? heard}) {
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    builder: (_) => _EntrySheet(store: ServicesScope.of(context).dictionary, heard: heard),
  );
}

class _EntrySheet extends StatefulWidget {
  const _EntrySheet({required this.store, this.heard});

  final DictionaryStore store;
  final String? heard;

  @override
  State<_EntrySheet> createState() => _EntrySheetState();
}

class _EntrySheetState extends State<_EntrySheet> {
  final _text = TextEditingController();
  late final _heard = TextEditingController(text: widget.heard);
  String? _error;

  @override
  void dispose() {
    _text.dispose();
    _heard.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    final DictionaryEntry entry;
    try {
      entry = DictionaryEntry(_text.text, heard: _heard.text);
    } on FormatException catch (error) {
      setState(() => _error = error.message);
      return;
    }
    if (!await widget.store.add(entry)) {
      setState(() => _error = 'This word is already in the dictionary.');
      return;
    }
    if (mounted) Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.fromLTRB(20, 0, 20, 20 + MediaQuery.viewInsetsOf(context).bottom),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Add word', style: Theme.of(context).textTheme.titleLarge),
          const SizedBox(height: 20),
          TextField(
            controller: _text,
            autofocus: true,
            textCapitalization: TextCapitalization.sentences,
            textInputAction: TextInputAction.next,
            decoration: InputDecoration(labelText: 'Spelling', errorText: _error),
            onChanged: (_) => setState(() => _error = null),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _heard,
            textInputAction: TextInputAction.done,
            decoration: const InputDecoration(labelText: 'Whisper hears (optional)'),
            onSubmitted: (_) => _save(),
          ),
          const SizedBox(height: 20),
          FilledButton(onPressed: _save, child: const Text('Add')),
        ],
      ),
    );
  }
}
