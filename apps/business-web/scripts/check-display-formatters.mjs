import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative, resolve } from "node:path";
import ts from "typescript";

const DISPLAY_FIELD_SUFFIXES = [
  "Amount",
  "Cost",
  "Price",
  "Profit",
  "Quantity",
  "Revenue",
];

const DISPLAY_EXACT_FIELDS = new Set([
  "currentInventoryValue",
  "inventoryValue",
  "projectedInventoryValue",
  "quantity",
  "varianceValue",
]);

const APPROVED_FORMATTERS = new Set([
  "formatAmount",
  "formatDecimal",
  "formatMoney",
  "formatQuantity",
  "formatSignedQuantity",
]);

function fieldName(expression) {
  return ts.isPropertyAccessExpression(expression) ? expression.name.text : null;
}

function isBusinessDecimalField(name) {
  return (
    DISPLAY_EXACT_FIELDS.has(name) ||
    DISPLAY_FIELD_SUFFIXES.some((suffix) => name.endsWith(suffix))
  );
}

function isApprovedFormatterCall(expression) {
  return (
    ts.isCallExpression(expression) &&
    ts.isIdentifier(expression.expression) &&
    APPROVED_FORMATTERS.has(expression.expression.text)
  );
}

function directBusinessFields(expression) {
  if (isApprovedFormatterCall(expression)) return [];

  if (ts.isPropertyAccessExpression(expression)) {
    const name = fieldName(expression);
    return name && isBusinessDecimalField(name) ? [name] : [];
  }

  if (
    ts.isParenthesizedExpression(expression) ||
    ts.isAsExpression(expression) ||
    ts.isNonNullExpression(expression)
  ) {
    return directBusinessFields(expression.expression);
  }

  if (ts.isBinaryExpression(expression)) {
    return [
      ...directBusinessFields(expression.left),
      ...directBusinessFields(expression.right),
    ];
  }

  if (ts.isConditionalExpression(expression)) {
    return [
      ...directBusinessFields(expression.whenTrue),
      ...directBusinessFields(expression.whenFalse),
    ];
  }

  return [];
}

export function findUnformattedBusinessDisplays(source, file = "source.tsx") {
  const sourceFile = ts.createSourceFile(
    file,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const violations = [];

  function visit(node) {
    if (
      ts.isJsxExpression(node) &&
      node.expression &&
      !ts.isJsxAttribute(node.parent)
    ) {
      for (const field of directBusinessFields(node.expression)) {
        const position = sourceFile.getLineAndCharacterOfPosition(
          node.expression.getStart(sourceFile),
        );
        violations.push({
          field,
          line: position.line + 1,
          column: position.character + 1,
        });
      }
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return violations;
}

function tsxFiles(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return tsxFiles(path);
    return entry.isFile() && entry.name.endsWith(".tsx") ? [path] : [];
  });
}

export function checkDirectory(directory) {
  return tsxFiles(directory).flatMap((file) =>
    findUnformattedBusinessDisplays(readFileSync(file, "utf8"), file).map(
      (violation) => ({ file, ...violation }),
    ),
  );
}

const scriptPath = fileURLToPath(import.meta.url);
if (process.argv[1] && resolve(process.argv[1]) === scriptPath) {
  const projectRoot = resolve(dirname(scriptPath), "..");
  const violations = checkDirectory(join(projectRoot, "src"));
  if (violations.length > 0) {
    console.error("发现未经共享格式函数处理的金额或数量展示：");
    for (const violation of violations) {
      console.error(
        `- ${relative(projectRoot, violation.file)}:${violation.line}:${violation.column} ${violation.field}`,
      );
    }
    console.error(
      "请使用 formatAmount、formatMoney、formatQuantity 或 formatSignedQuantity。",
    );
    process.exitCode = 1;
  } else {
    console.log("金额与数量展示格式巡检通过。");
  }
}
