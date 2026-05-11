import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:torex_local_store/torex_local_store.dart';

// ─────────────────────────────────────────────────────────────────────────────
// CrudPage — Ma'lumot kiritish, ko'rish, tahrirlash va o'chirish sahifasi
// ─────────────────────────────────────────────────────────────────────────────

class CrudPage extends StatefulWidget {
  const CrudPage({super.key});

  @override
  State<CrudPage> createState() => _CrudPageState();
}

class _CrudPageState extends State<CrudPage> {
  // ── State ──────────────────────────────────────────────────────────────────
  String _collection = 'my_data';
  List<_Record> _records = [];
  List<_Record> _filtered = [];
  bool _loading = false;
  String? _editingKey; // null = yangi qo'shish, String = tahrirlash

  // ── Controllers ────────────────────────────────────────────────────────────
  final _keyCtrl = TextEditingController();
  final _valueCtrl = TextEditingController();
  final _searchCtrl = TextEditingController();
  final _collectionCtrl = TextEditingController(text: 'my_data');
  final _keyFocus = FocusNode();

  // ── Preset collections ────────────────────────────────────────────────────
  final List<String> _presets = [
    'my_data',
    'users',
    'settings',
    'products',
    'orders',
  ];

  @override
  void initState() {
    super.initState();
    _searchCtrl.addListener(_onSearch);
    _load();
  }

  @override
  void dispose() {
    _keyCtrl.dispose();
    _valueCtrl.dispose();
    _searchCtrl.dispose();
    _collectionCtrl.dispose();
    _keyFocus.dispose();
    super.dispose();
  }

  // ── Data helpers ───────────────────────────────────────────────────────────

  Future<void> _load() async {
    setState(() => _loading = true);
    try {
      final box = Torex.box(_collection);
      final entries = await box.scanStrings();
      setState(() {
        _records = entries.map((e) => _Record(e.$1, e.$2)).toList();
        _applyFilter();
        _loading = false;
      });
    } catch (e) {
      setState(() => _loading = false);
      _showError('Yuklashda xato: $e');
    }
  }

  void _onSearch() => setState(_applyFilter);

  void _applyFilter() {
    final q = _searchCtrl.text.trim().toLowerCase();
    _filtered = q.isEmpty
        ? List.from(_records)
        : _records
            .where(
              (r) =>
                  r.key.toLowerCase().contains(q) ||
                  r.value.toLowerCase().contains(q),
            )
            .toList();
  }

  // ── CRUD operations ────────────────────────────────────────────────────────

  Future<void> _save() async {
    final key = _keyCtrl.text.trim();
    final value = _valueCtrl.text.trim();
    if (key.isEmpty) {
      _showError('Kalit bo\'sh bo\'lmasligi kerak');
      return;
    }

    setState(() => _loading = true);
    try {
      await Torex.box(_collection).put(key, value);
      _clearForm();
      await _load();
      _showSnack(
        _editingKey == null ? '✅ Qo\'shildi: "$key"' : '✅ Yangilandi: "$key"',
        Colors.green,
      );
    } catch (e) {
      setState(() => _loading = false);
      _showError('Saqlashda xato: $e');
    }
  }

  Future<void> _delete(String key) async {
    final ok = await _confirm(
      'O\'chirish',
      '"$key" kalitini o\'chirmoqchimisiz?',
    );
    if (!ok) return;

    setState(() => _loading = true);
    try {
      await Torex.box(_collection).delete(key);
      if (_editingKey == key) _clearForm();
      await _load();
      _showSnack('🗑 O\'chirildi: "$key"', Colors.orange);
    } catch (e) {
      setState(() => _loading = false);
      _showError('O\'chirishda xato: $e');
    }
  }

  Future<void> _clearAll() async {
    if (_records.isEmpty) return;
    final ok = await _confirm(
      'Hammasini o\'chirish',
      '"$_collection" kolleksiyasidagi ${_records.length} ta yozuv o\'chiriladi. Davom etasizmi?',
    );
    if (!ok) return;

    setState(() => _loading = true);
    try {
      await Torex.box(_collection).clear();
      _clearForm();
      await _load();
      _showSnack('🗑 Kolleksiya tozalandi', Colors.red);
    } catch (e) {
      setState(() => _loading = false);
      _showError('Tozalashda xato: $e');
    }
  }

  void _startEdit(_Record record) {
    setState(() {
      _editingKey = record.key;
      _keyCtrl.text = record.key;
      _valueCtrl.text = record.value;
    });
    _keyFocus.requestFocus();
    // Formga scroll qilish
    Scrollable.ensureVisible(
      _keyFocus.context ?? context,
      duration: const Duration(milliseconds: 300),
    );
  }

  void _clearForm() {
    _editingKey = null;
    _keyCtrl.clear();
    _valueCtrl.clear();
  }

  Future<void> _switchCollection(String name) async {
    if (name == _collection) return;
    setState(() {
      _collection = name;
      _collectionCtrl.text = name;
      _clearForm();
      _searchCtrl.clear();
      _records = [];
      _filtered = [];
    });
    await _load();
  }

  // ── UI helpers ─────────────────────────────────────────────────────────────

  void _showSnack(String msg, Color color) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(msg, style: const TextStyle(fontWeight: FontWeight.w600)),
        backgroundColor: color,
        behavior: SnackBarBehavior.floating,
        duration: const Duration(seconds: 2),
      ),
    );
  }

  void _showError(String msg) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(
        content: Text(msg),
        backgroundColor: Colors.red,
        behavior: SnackBarBehavior.floating,
      ),
    );
  }

  Future<bool> _confirm(String title, String message) async {
    return await showDialog<bool>(
          context: context,
          builder: (ctx) => AlertDialog(
            title: Text(title),
            content: Text(message),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(ctx, false),
                child: const Text('Bekor'),
              ),
              FilledButton(
                onPressed: () => Navigator.pop(ctx, true),
                style: FilledButton.styleFrom(backgroundColor: Colors.red),
                child: const Text('O\'chirish'),
              ),
            ],
          ),
        ) ??
        false;
  }

  // ── Build ──────────────────────────────────────────────────────────────────

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final cs = theme.colorScheme;

    return Scaffold(
      backgroundColor: cs.surface,
      appBar: _buildAppBar(cs),
      body: Column(
        children: [
          _buildCollectionBar(cs),
          _buildForm(cs),
          _buildSearchBar(cs),
          _buildListHeader(cs),
          Expanded(child: _buildList(cs)),
        ],
      ),
    );
  }

  // ── AppBar ─────────────────────────────────────────────────────────────────

  PreferredSizeWidget _buildAppBar(ColorScheme cs) {
    return AppBar(
      backgroundColor: cs.inversePrimary,
      title: Row(
        children: [
          const Icon(Icons.storage_rounded, size: 22),
          const SizedBox(width: 8),
          const Text('Ma\'lumot Boshqaruvi'),
        ],
      ),
      actions: [
        // Record count badge
        Container(
          margin: const EdgeInsets.only(right: 4),
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
          decoration: BoxDecoration(
            color: cs.primary,
            borderRadius: BorderRadius.circular(20),
          ),
          child: Text(
            '${_records.length} ta',
            style: TextStyle(
              color: cs.onPrimary,
              fontSize: 12,
              fontWeight: FontWeight.bold,
            ),
          ),
        ),
        IconButton(
          icon: const Icon(Icons.refresh_rounded),
          tooltip: 'Yangilash',
          onPressed: _loading ? null : _load,
        ),
        if (_records.isNotEmpty)
          IconButton(
            icon: const Icon(Icons.delete_sweep_rounded),
            tooltip: 'Hammasini o\'chirish',
            onPressed: _loading ? null : _clearAll,
          ),
      ],
    );
  }

  // ── Collection selector bar ────────────────────────────────────────────────

  Widget _buildCollectionBar(ColorScheme cs) {
    return Container(
      padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
      color: cs.surfaceContainerHighest.withValues(alpha: 0.4),
      child: Row(
        children: [
          const Icon(Icons.folder_rounded, size: 16),
          const SizedBox(width: 6),
          const Text(
            'Kolleksiya:',
            style: TextStyle(fontSize: 13, fontWeight: FontWeight.w600),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: <Widget>[..._presets.map((name) {
                  final active = name == _collection;
                  return Padding(
                    padding: const EdgeInsets.only(right: 6),
                    child: GestureDetector(
                      onTap: () => _switchCollection(name),
                      child: AnimatedContainer(
                        duration: const Duration(milliseconds: 200),
                        padding: const EdgeInsets.symmetric(
                          horizontal: 12,
                          vertical: 5,
                        ),
                        decoration: BoxDecoration(
                          color: active ? cs.primary : cs.surfaceContainerHighest,
                          borderRadius: BorderRadius.circular(20),
                          border: Border.all(
                            color: active ? cs.primary : cs.outline.withValues(alpha: 0.4),
                          ),
                        ),
                        child: Text(
                          name,
                          style: TextStyle(
                            fontSize: 12,
                            fontWeight: active ? FontWeight.bold : FontWeight.normal,
                            color: active ? cs.onPrimary : cs.onSurface,
                          ),
                        ),
                      ),
                    ),
                  );
                }), _buildCustomCollectionChip(cs)],
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildCustomCollectionChip(ColorScheme cs) {
    return GestureDetector(
      onTap: _showCustomCollectionDialog,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 5),
        decoration: BoxDecoration(
          border: Border.all(
            color: cs.outline.withValues(alpha: 0.5),
            style: BorderStyle.solid,
          ),
          borderRadius: BorderRadius.circular(20),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.add, size: 14, color: cs.primary),
            const SizedBox(width: 4),
            Text(
              'Yangi',
              style: TextStyle(fontSize: 12, color: cs.primary),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _showCustomCollectionDialog() async {
    final ctrl = TextEditingController();
    final name = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text('Yangi kolleksiya'),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          decoration: const InputDecoration(
            hintText: 'Kolleksiya nomi',
            prefixIcon: Icon(Icons.folder_open_rounded),
          ),
          onSubmitted: (v) => Navigator.pop(ctx, v.trim()),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text('Bekor'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, ctrl.text.trim()),
            child: const Text('Ochish'),
          ),
        ],
      ),
    );
    if (name != null && name.isNotEmpty) {
      _presets.add(name);
      await _switchCollection(name);
    }
  }

  // ── Add / Edit form ────────────────────────────────────────────────────────

  Widget _buildForm(ColorScheme cs) {
    final isEditing = _editingKey != null;

    return AnimatedContainer(
      duration: const Duration(milliseconds: 250),
      margin: const EdgeInsets.fromLTRB(12, 8, 12, 4),
      decoration: BoxDecoration(
        color: isEditing
            ? cs.secondaryContainer.withValues(alpha: 0.5)
            : cs.surfaceContainerLowest,
        borderRadius: BorderRadius.circular(16),
        border: Border.all(
          color: isEditing ? cs.secondary : cs.outlineVariant,
          width: isEditing ? 1.5 : 1,
        ),
      ),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Form header
            Row(
              children: [
                Icon(
                  isEditing ? Icons.edit_rounded : Icons.add_circle_rounded,
                  size: 18,
                  color: isEditing ? cs.secondary : cs.primary,
                ),
                const SizedBox(width: 6),
                Text(
                  isEditing ? 'Tahrirlash: "${_editingKey!}"' : 'Yangi yozuv qo\'shish',
                  style: TextStyle(
                    fontSize: 13,
                    fontWeight: FontWeight.w700,
                    color: isEditing ? cs.secondary : cs.primary,
                  ),
                ),
                if (isEditing) ...[
                  const Spacer(),
                  GestureDetector(
                    onTap: () => setState(_clearForm),
                    child: Icon(Icons.close, size: 18, color: cs.error),
                  ),
                ],
              ],
            ),
            const SizedBox(height: 10),

            // Key field
            TextField(
              controller: _keyCtrl,
              focusNode: _keyFocus,
              readOnly: isEditing, // tahrirlayotganda kalit o'zgarmasin
              decoration: InputDecoration(
                labelText: 'Kalit (Key)',
                hintText: 'Masalan: user:123',
                prefixIcon: const Icon(Icons.key_rounded, size: 18),
                isDense: true,
                filled: true,
                fillColor: isEditing
                    ? cs.surfaceContainerHighest.withValues(alpha: 0.4)
                    : null,
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(10),
                ),
              ),
              textInputAction: TextInputAction.next,
            ),
            const SizedBox(height: 8),

            // Value field
            TextField(
              controller: _valueCtrl,
              maxLines: 3,
              minLines: 1,
              decoration: InputDecoration(
                labelText: 'Qiymat (Value)',
                hintText: 'Masalan: {"name": "Ali"}',
                prefixIcon: const Icon(Icons.data_object_rounded, size: 18),
                isDense: true,
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(10),
                ),
                // Copy button
                suffixIcon: _valueCtrl.text.isNotEmpty
                    ? IconButton(
                        icon: const Icon(Icons.copy_rounded, size: 16),
                        tooltip: 'Nusxalash',
                        onPressed: () {
                          Clipboard.setData(
                            ClipboardData(text: _valueCtrl.text),
                          );
                          _showSnack('📋 Nusxalandi', Colors.blueGrey);
                        },
                      )
                    : null,
              ),
              onChanged: (_) => setState(() {}),
            ),
            const SizedBox(height: 10),

            // Action buttons
            Row(
              children: [
                if (isEditing) ...[
                  OutlinedButton.icon(
                    onPressed: () => setState(_clearForm),
                    icon: const Icon(Icons.close, size: 16),
                    label: const Text('Bekor'),
                  ),
                  const SizedBox(width: 8),
                ],
                Expanded(
                  child: FilledButton.icon(
                    onPressed: _loading ? null : _save,
                    icon: _loading
                        ? const SizedBox(
                            width: 16,
                            height: 16,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : Icon(
                            isEditing ? Icons.save_rounded : Icons.add_rounded,
                            size: 18,
                          ),
                    label: Text(isEditing ? 'Saqlash' : 'Qo\'shish'),
                    style: FilledButton.styleFrom(
                      backgroundColor: isEditing ? cs.secondary : cs.primary,
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  // ── Search bar ─────────────────────────────────────────────────────────────

  Widget _buildSearchBar(ColorScheme cs) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 4, 12, 4),
      child: TextField(
        controller: _searchCtrl,
        decoration: InputDecoration(
          hintText: 'Qidirish (kalit yoki qiymat bo\'yicha)...',
          prefixIcon: const Icon(Icons.search_rounded, size: 20),
          isDense: true,
          filled: true,
          fillColor: cs.surfaceContainerHighest.withValues(alpha: 0.5),
          border: OutlineInputBorder(
            borderRadius: BorderRadius.circular(12),
            borderSide: BorderSide.none,
          ),
          suffixIcon: _searchCtrl.text.isNotEmpty
              ? IconButton(
                  icon: const Icon(Icons.clear_rounded, size: 18),
                  onPressed: () {
                    _searchCtrl.clear();
                    setState(_applyFilter);
                  },
                )
              : null,
        ),
      ),
    );
  }

  // ── List header ────────────────────────────────────────────────────────────

  Widget _buildListHeader(ColorScheme cs) {
    return Container(
      padding: const EdgeInsets.fromLTRB(16, 4, 16, 4),
      child: Row(
        children: [
          Text(
            _searchCtrl.text.isNotEmpty
                ? '${_filtered.length} ta natija'
                : '${_records.length} ta yozuv',
            style: TextStyle(
              fontSize: 12,
              color: cs.onSurface.withValues(alpha: 0.6),
              fontWeight: FontWeight.w600,
            ),
          ),
          const Spacer(),
          Text(
            'Kalit',
            style: TextStyle(
              fontSize: 11,
              color: cs.onSurface.withValues(alpha: 0.4),
            ),
          ),
          const SizedBox(width: 60),
          Text(
            'Qiymat',
            style: TextStyle(
              fontSize: 11,
              color: cs.onSurface.withValues(alpha: 0.4),
            ),
          ),
          const SizedBox(width: 72),
        ],
      ),
    );
  }

  // ── Records list ───────────────────────────────────────────────────────────

  Widget _buildList(ColorScheme cs) {
    if (_loading && _records.isEmpty) {
      return const Center(child: CircularProgressIndicator());
    }

    if (_filtered.isEmpty) {
      return _buildEmptyState(cs);
    }

    return ListView.builder(
      padding: const EdgeInsets.fromLTRB(12, 0, 12, 80),
      itemCount: _filtered.length,
      itemBuilder: (ctx, i) => _buildRecordTile(_filtered[i], cs),
    );
  }

  Widget _buildEmptyState(ColorScheme cs) {
    final hasSearch = _searchCtrl.text.isNotEmpty;
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            hasSearch ? Icons.search_off_rounded : Icons.inbox_rounded,
            size: 64,
            color: cs.onSurface.withValues(alpha: 0.2),
          ),
          const SizedBox(height: 12),
          Text(
            hasSearch
                ? '"${_searchCtrl.text}" bo\'yicha natija topilmadi'
                : '"$_collection" bo\'sh\nYuqoridagi forma orqali ma\'lumot kiriting',
            textAlign: TextAlign.center,
            style: TextStyle(
              color: cs.onSurface.withValues(alpha: 0.4),
              fontSize: 14,
            ),
          ),
        ],
      ),
    );
  }

  Widget _buildRecordTile(_Record record, ColorScheme cs) {
    final isSelected = _editingKey == record.key;

    return Dismissible(
      key: ValueKey(record.key),
      direction: DismissDirection.endToStart,
      background: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 20),
        margin: const EdgeInsets.only(bottom: 6),
        decoration: BoxDecoration(
          color: cs.error,
          borderRadius: BorderRadius.circular(12),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(Icons.delete_rounded, color: cs.onError),
            Text(
              'O\'chirish',
              style: TextStyle(color: cs.onError, fontSize: 11),
            ),
          ],
        ),
      ),
      confirmDismiss: (_) => _confirm(
        'O\'chirish',
        '"${record.key}" kalitini o\'chirmoqchimisiz?',
      ),
      onDismissed: (_) => _delete(record.key),
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 200),
        margin: const EdgeInsets.only(bottom: 6),
        decoration: BoxDecoration(
          color: isSelected
              ? cs.secondaryContainer.withValues(alpha: 0.6)
              : cs.surfaceContainerLowest,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(
            color: isSelected ? cs.secondary : cs.outlineVariant.withValues(alpha: 0.5),
            width: isSelected ? 1.5 : 1,
          ),
        ),
        child: ListTile(
          dense: true,
          contentPadding: const EdgeInsets.fromLTRB(12, 4, 8, 4),
          // Key
          title: Row(
            children: [
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
                decoration: BoxDecoration(
                  color: cs.primaryContainer,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text(
                  record.key,
                  style: TextStyle(
                    fontSize: 12,
                    fontWeight: FontWeight.w700,
                    color: cs.onPrimaryContainer,
                    fontFamily: 'monospace',
                  ),
                  overflow: TextOverflow.ellipsis,
                  maxLines: 1,
                ),
              ),
            ],
          ),
          // Value
          subtitle: Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              record.value,
              style: TextStyle(
                fontSize: 12,
                color: cs.onSurface.withValues(alpha: 0.7),
                fontFamily: 'monospace',
              ),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          // Actions
          trailing: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              // Copy value
              IconButton(
                icon: Icon(
                  Icons.copy_rounded,
                  size: 18,
                  color: cs.onSurface.withValues(alpha: 0.4),
                ),
                tooltip: 'Qiymatni nusxalash',
                onPressed: () {
                  Clipboard.setData(ClipboardData(text: record.value));
                  _showSnack('📋 Nusxalandi', Colors.blueGrey);
                },
              ),
              // Edit
              IconButton(
                icon: Icon(
                  Icons.edit_rounded,
                  size: 18,
                  color: isSelected ? cs.secondary : cs.primary,
                ),
                tooltip: 'Tahrirlash',
                onPressed: () => _startEdit(record),
              ),
              // Delete
              IconButton(
                icon: Icon(
                  Icons.delete_rounded,
                  size: 18,
                  color: cs.error,
                ),
                tooltip: 'O\'chirish',
                onPressed: () => _delete(record.key),
              ),
            ],
          ),
          onTap: () => _startEdit(record),
          onLongPress: () {
            Clipboard.setData(
              ClipboardData(text: '${record.key}: ${record.value}'),
            );
            _showSnack('📋 Kalit va qiymat nusxalandi', Colors.blueGrey);
          },
        ),
      ),
    );
  }
}

// ─── Helper model ─────────────────────────────────────────────────────────────

class _Record {
  final String key;
  final String value;
  const _Record(this.key, this.value);
}
