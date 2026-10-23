import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../history.dart';
import '../models.dart';
import '../services.dart';
import 'format.dart';
import 'onboarding_screen.dart';
import 'theme.dart';
import 'widgets.dart';

class HistoryScreen extends StatefulWidget {
  const HistoryScreen({super.key});

  @override
  State<HistoryScreen> createState() => _HistoryScreenState();
}

class _HistoryScreenState extends State<HistoryScreen> {
  final _query = TextEditingController();
  bool _searching = false;

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    return ListenableBuilder(
      listenable: Listenable.merge([services.history, _query]),
      builder: (context, _) {
        final query = _query.text.trim().toLowerCase();
        final items = services.history.items
            .where((item) => query.isEmpty || item.text.toLowerCase().contains(query))
            .toList();
        return CustomScrollView(
          slivers: [
            SliverToBoxAdapter(child: _header(services)),
            const SliverToBoxAdapter(child: _StatusBanner()),
            if (items.isEmpty)
              SliverFillRemaining(
                hasScrollBody: false,
                child: EmptyState(
                  services.history.items.isEmpty
                      ? 'Your dictations show up here.'
                      : 'Nothing matches.',
                ),
              )
            else
              SliverPadding(
                padding: const EdgeInsets.fromLTRB(16, 0, 16, 24),
                sliver: SliverList.builder(
                  itemCount: items.length,
                  itemBuilder: (context, index) {
                    final item = items[index];
                    final newDay = index == 0 || !sameDay(items[index - 1].at, item.at);
                    return Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        if (newDay)
                          Padding(
                            padding: EdgeInsets.fromLTRB(4, index == 0 ? 4 : 16, 4, 8),
                            child: Text(
                              dayLabel(item.at, DateTime.now()).toUpperCase(),
                              style: Theme.of(context).textTheme.labelSmall,
                            ),
                          ),
                        _HistoryTile(item: item),
                      ],
                    );
                  },
                ),
              ),
          ],
        );
      },
    );
  }

  Widget _header(Services services) {
    if (_searching) {
      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
        child: Row(
          children: [
            Expanded(
              child: TextField(
                controller: _query,
                autofocus: true,
                textInputAction: TextInputAction.search,
                decoration: const InputDecoration(
                  hintText: 'Search history',
                  prefixIcon: Icon(Icons.search_rounded),
                ),
              ),
            ),
            IconButton(
              tooltip: 'Close search',
              icon: const Icon(Icons.close_rounded),
              onPressed: () => setState(() {
                _searching = false;
                _query.clear();
              }),
            ),
          ],
        ),
      );
    }
    final empty = services.history.items.isEmpty;
    return PageHeader(
      'History',
      actions: [
        if (!empty)
          IconButton(
            tooltip: 'Search',
            icon: const Icon(Icons.search_rounded),
            onPressed: () => setState(() => _searching = true),
          ),
        if (!empty)
          PopupMenuButton<void>(
            tooltip: 'More',
            icon: const Icon(Icons.more_vert_rounded),
            color: Palette.raised,
            itemBuilder: (context) => [
              PopupMenuItem(
                onTap: () => _confirmClear(services),
                child: const Text('Clear history'),
              ),
            ],
          ),
      ],
    );
  }

  Future<void> _confirmClear(Services services) async {
    final clear = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Clear history?'),
        content: const Text('This deletes every dictation on this phone. It cannot be undone.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          TextButton(
            style: TextButton.styleFrom(foregroundColor: Palette.danger),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Clear'),
          ),
        ],
      ),
    );
    if (clear == true) await services.history.clear();
  }
}

class _HistoryTile extends StatelessWidget {
  const _HistoryTile({required this.item});

  final HistoryItem item;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Dismissible(
        key: ValueKey(item.id),
        direction: DismissDirection.endToStart,
        background: Container(
          alignment: Alignment.centerRight,
          padding: const EdgeInsets.only(right: 24),
          decoration: BoxDecoration(
            color: Palette.danger.withValues(alpha: 0.16),
            borderRadius: BorderRadius.circular(20),
          ),
          child: const Icon(Icons.delete_outline_rounded, color: Palette.danger),
        ),
        onDismissed: (_) => _delete(context, item),
        child: SurfaceCard(
          child: InkWell(
            onTap: () => _showDetails(context, item),
            onLongPress: () => _copy(context, item.text),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(16, 14, 16, 12),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    item.text,
                    maxLines: 4,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontSize: 15.5, height: 1.45),
                  ),
                  const SizedBox(height: 10),
                  _Meta(item: item),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Meta extends StatelessWidget {
  const _Meta({required this.item});

  final HistoryItem item;

  @override
  Widget build(BuildContext context) {
    final system = ServicesScope.of(context).system;
    const meta = TextStyle(fontFamily: mono, fontSize: 12, color: Palette.faint);
    // A wrap, so large text moves the tag to its own line instead of overflowing.
    return Wrap(
      spacing: 10,
      runSpacing: 6,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        if (item.app != null)
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              AppIcon(item.app!, size: 16),
              const SizedBox(width: 8),
              ListenableBuilder(
                listenable: system,
                builder: (context, _) => Text(
                  system.app(item.app!)?.name ?? '',
                  style: const TextStyle(fontSize: 12.5, color: Palette.muted),
                ),
              ),
            ],
          ),
        Text(clock(item.at), style: meta),
        Text(seconds(item.audioMs), style: meta),
        if (item.delivered == false) const Tag('Not inserted'),
      ],
    );
  }
}

void _delete(BuildContext context, HistoryItem item) {
  final history = ServicesScope.of(context).history;
  history.remove(item.id);
  ScaffoldMessenger.of(context)
    ..hideCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        content: const Text('Dictation deleted'),
        action: SnackBarAction(label: 'Undo', onPressed: () => history.restore(item)),
      ),
    );
}

void _copy(BuildContext context, String text) {
  Clipboard.setData(ClipboardData(text: text));
  ScaffoldMessenger.of(context)
    ..hideCurrentSnackBar()
    ..showSnackBar(const SnackBar(content: Text('Copied')));
}

void _showDetails(BuildContext context, HistoryItem item) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    builder: (sheet) => SafeArea(
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: MediaQuery.sizeOf(sheet).height * 0.8),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(20, 0, 20, 16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                '${dayLabel(item.at, DateTime.now())}, ${clock(item.at)}',
                style: Theme.of(sheet).textTheme.labelSmall,
              ),
              const SizedBox(height: 12),
              Flexible(
                child: SingleChildScrollView(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      SelectableText(item.text, style: const TextStyle(fontSize: 17, height: 1.5)),
                      if (item.transcript != item.text) ...[
                        const SizedBox(height: 16),
                        Text('WHISPER HEARD', style: Theme.of(sheet).textTheme.labelSmall),
                        const SizedBox(height: 6),
                        SelectableText(
                          item.transcript,
                          style: const TextStyle(color: Palette.muted, height: 1.45),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 20),
              Row(
                children: [
                  Expanded(
                    child: OutlinedButton.icon(
                      icon: const Icon(Icons.delete_outline_rounded, size: 20),
                      style: OutlinedButton.styleFrom(foregroundColor: Palette.danger),
                      label: const Text('Delete'),
                      onPressed: () {
                        Navigator.pop(sheet);
                        _delete(context, item);
                      },
                    ),
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: FilledButton.icon(
                      icon: const Icon(Icons.copy_rounded, size: 20),
                      label: const Text('Copy'),
                      onPressed: () {
                        Navigator.pop(sheet);
                        _copy(context, item.text);
                      },
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    ),
  );
}

/// Tells why the bubble would not work right now: setup left to do, or a snooze.
class _StatusBanner extends StatelessWidget {
  const _StatusBanner();

  @override
  Widget build(BuildContext context) {
    final services = ServicesScope.of(context);
    return ListenableBuilder(
      listenable: Listenable.merge([services.system, services.settings, services.models]),
      builder: (context, _) {
        final system = services.system;
        if (!system.loaded) return const SizedBox.shrink();
        final model = modelByFile(services.settings.settings.model);
        final missing = [
          if (!system.status.microphone) 'microphone',
          if (!system.status.service) 'bubble',
          if (model == null || !services.models.isDownloaded(model)) 'model',
        ];
        if (missing.isNotEmpty) {
          return _Banner(
            icon: Icons.error_outline_rounded,
            title: 'Finish setup',
            subtitle: 'Still needed: ${missing.join(', ')}',
            action: 'Open',
            onPressed: () =>
                Navigator.of(context)
                    .push(MaterialPageRoute<void>(builder: (_) => const OnboardingScreen())),
          );
        }
        final now = DateTime.now();
        final snoozed = system.bubble.snoozedUntil;
        if (snoozed != null && snoozed.isAfter(now)) {
          return _Banner(
            icon: Icons.bedtime_outlined,
            title: 'Bubble paused',
            subtitle: 'Hidden ${until(snoozed, now)}',
            action: 'Resume',
            onPressed: system.resume,
          );
        }
        return const SizedBox.shrink();
      },
    );
  }
}

class _Banner extends StatelessWidget {
  const _Banner({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.action,
    required this.onPressed,
  });

  final IconData icon;
  final String title;
  final String subtitle;
  final String action;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 4, 16, 16),
      child: SurfaceCard(
        color: Palette.emberWash,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 8, 12),
          child: Row(
            children: [
              Icon(icon, color: Palette.ember),
              const SizedBox(width: 14),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(title, style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w600)),
                    const SizedBox(height: 2),
                    Text(subtitle, style: const TextStyle(fontSize: 13, color: Palette.muted)),
                  ],
                ),
              ),
              TextButton(onPressed: onPressed, child: Text(action)),
            ],
          ),
        ),
      ),
    );
  }
}
