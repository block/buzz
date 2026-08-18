import 'package:flutter/material.dart';
import 'package:highlight/highlight.dart' show highlight, Node;

const highlightLightTheme = <String, TextStyle>{
  'keyword': TextStyle(color: Color(0xFFa626a4)),
  'built_in': TextStyle(color: Color(0xFF0184bc)),
  'type': TextStyle(color: Color(0xFF0184bc)),
  'literal': TextStyle(color: Color(0xFF0184bc)),
  'number': TextStyle(color: Color(0xFF986801)),
  'string': TextStyle(color: Color(0xFF50a14f)),
  'symbol': TextStyle(color: Color(0xFF50a14f)),
  'comment': TextStyle(color: Color(0xFFa0a1a7), fontStyle: FontStyle.italic),
  'doctag': TextStyle(color: Color(0xFFa0a1a7), fontStyle: FontStyle.italic),
  'meta': TextStyle(color: Color(0xFF986801)),
  'attr': TextStyle(color: Color(0xFF986801)),
  'attribute': TextStyle(color: Color(0xFF986801)),
  'title': TextStyle(color: Color(0xFF4078f2)),
  'title.class_': TextStyle(color: Color(0xFFc18401)),
  'title.function_': TextStyle(color: Color(0xFF4078f2)),
  'name': TextStyle(color: Color(0xFFe45649)),
  'tag': TextStyle(color: Color(0xFFe45649)),
  'selector-tag': TextStyle(color: Color(0xFFe45649)),
  'params': TextStyle(color: Color(0xFF383a42)),
  'variable': TextStyle(color: Color(0xFFe45649)),
  'subst': TextStyle(color: Color(0xFFe45649)),
  'section': TextStyle(color: Color(0xFF4078f2)),
  'bullet': TextStyle(color: Color(0xFF4078f2)),
  'link': TextStyle(color: Color(0xFF4078f2)),
  'addition': TextStyle(color: Color(0xFF50a14f)),
  'deletion': TextStyle(color: Color(0xFFe45649)),
};

const highlightDarkTheme = <String, TextStyle>{
  'keyword': TextStyle(color: Color(0xFFc678dd)),
  'built_in': TextStyle(color: Color(0xFF56b6c2)),
  'type': TextStyle(color: Color(0xFF56b6c2)),
  'literal': TextStyle(color: Color(0xFF56b6c2)),
  'number': TextStyle(color: Color(0xFFd19a66)),
  'string': TextStyle(color: Color(0xFF98c379)),
  'symbol': TextStyle(color: Color(0xFF98c379)),
  'comment': TextStyle(color: Color(0xFF5c6370), fontStyle: FontStyle.italic),
  'doctag': TextStyle(color: Color(0xFF5c6370), fontStyle: FontStyle.italic),
  'meta': TextStyle(color: Color(0xFFd19a66)),
  'attr': TextStyle(color: Color(0xFFd19a66)),
  'attribute': TextStyle(color: Color(0xFFd19a66)),
  'title': TextStyle(color: Color(0xFF61afef)),
  'title.class_': TextStyle(color: Color(0xFFe5c07b)),
  'title.function_': TextStyle(color: Color(0xFF61afef)),
  'name': TextStyle(color: Color(0xFFe06c75)),
  'tag': TextStyle(color: Color(0xFFe06c75)),
  'selector-tag': TextStyle(color: Color(0xFFe06c75)),
  'params': TextStyle(color: Color(0xFFabb2bf)),
  'variable': TextStyle(color: Color(0xFFe06c75)),
  'subst': TextStyle(color: Color(0xFFe06c75)),
  'section': TextStyle(color: Color(0xFF61afef)),
  'bullet': TextStyle(color: Color(0xFF61afef)),
  'link': TextStyle(color: Color(0xFF61afef)),
  'addition': TextStyle(color: Color(0xFF98c379)),
  'deletion': TextStyle(color: Color(0xFFe06c75)),
};

List<InlineSpan> highlightCode(
  String code,
  String language,
  Map<String, TextStyle> theme,
  TextStyle baseStyle,
) {
  try {
    if (language.isEmpty) return [TextSpan(text: code, style: baseStyle)];
    final result = highlight.parse(code, language: language);
    if (result.nodes == null) return [TextSpan(text: code, style: baseStyle)];
    return buildSpans(result.nodes!, theme, baseStyle);
  } catch (_) {
    return [TextSpan(text: code, style: baseStyle)];
  }
}

List<InlineSpan> buildSpans(
  List<Node> nodes,
  Map<String, TextStyle> theme,
  TextStyle baseStyle, {
  int maxDepth = 10,
}) {
  final spans = <InlineSpan>[];
  for (final node in nodes) {
    if (maxDepth <= 0) {
      if (node.value != null) {
        spans.add(TextSpan(text: node.value, style: baseStyle));
      }
      continue;
    }
    if (node.children != null && node.children!.isNotEmpty) {
      final childStyle = node.className != null
          ? baseStyle.merge(theme[node.className])
          : baseStyle;
      spans.addAll(
        buildSpans(node.children!, theme, childStyle, maxDepth: maxDepth - 1),
      );
    } else if (node.value != null) {
      final style = node.className != null
          ? baseStyle.merge(theme[node.className])
          : baseStyle;
      spans.add(TextSpan(text: node.value, style: style));
    }
  }
  return spans;
}
