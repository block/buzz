import 'package:buzz/shared/widgets/collapsible_sliver_body.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

const _rowCount = 10;

/// Labels of row nodes a screen reader can reach (present and not hidden).
Set<String> _reachableRows(WidgetTester tester) {
  final reachable = <String>{};
  void visit(SemanticsNode node) {
    if (node.label.startsWith('Row ') && !node.flagsCollection.isHidden) {
      reachable.add(node.label);
    }
    node.visitChildren((child) {
      visit(child);
      return true;
    });
  }

  visit(tester.getSemantics(find.byKey(const Key('root'))));
  return reachable;
}

void main() {
  testWidgets('screen readers reach only rows inside the opening clip', (
    tester,
  ) async {
    final handle = tester.ensureSemantics();
    var expanded = false;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Semantics(
            key: const Key('root'),
            explicitChildNodes: true,
            child: StatefulBuilder(
              builder: (context, setState) => CustomScrollView(
                slivers: [
                  SliverToBoxAdapter(
                    child: TextButton(
                      onPressed: () => setState(() => expanded = !expanded),
                      child: const Text('Toggle'),
                    ),
                  ),
                  CollapsibleSliverBody(
                    expanded: expanded,
                    sliver: SliverList.builder(
                      itemCount: _rowCount,
                      itemBuilder: (_, index) => Semantics(
                        key: ValueKey('row-$index'),
                        button: true,
                        label: 'Row $index',
                        excludeSemantics: true,
                        child: const SizedBox(height: 48),
                      ),
                    ),
                  ),
                  const SliverToBoxAdapter(
                    child: SizedBox(
                      height: 100,
                      child: Text('Following section'),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('Toggle'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 16));

    final clipBottom = tester.getTopLeft(find.text('Following section')).dy;
    final reachable = _reachableRows(tester);
    for (var i = 0; i < _rowCount; i++) {
      final rowTop = tester.getTopLeft(find.byKey(ValueKey('row-$i'))).dy;
      expect(
        reachable.contains('Row $i'),
        rowTop < clipBottom,
        reason: 'Row $i top=$rowTop, painted extent ends at $clipBottom',
      );
    }
    expect(reachable, {for (var i = 0; i < 4; i++) 'Row $i'});

    await tester.pumpAndSettle();
    expect(_reachableRows(tester), {
      for (var i = 0; i < _rowCount; i++) 'Row $i',
    });

    handle.dispose();
  });
}
