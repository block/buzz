import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const root = path.resolve(import.meta.dirname, "../src");
const keys = JSON.parse(
  fs.readFileSync(path.join(root, "locales/ui-keys.json"), "utf8"),
);
const shape = (s) => s.replace(/\s*(\{\d+\})\s*/g, "$1");
const byShape = new Map(Object.keys(keys).map((s) => [shape(s), s]));
const write = process.argv.includes("--write");
const visible = new Set([
  "label",
  "title",
  "headerSubtitle",
  "description",
  "placeholder",
  "aria-label",
  "ariaLabel",
  "tooltip",
  "hint",
  "helpText",
  "emptyMessage",
  "subcopy",
  "alt",
  "legend",
  "message",
  "browseLabel",
  "createLabel",
  "chooseFromCatalog",
  "createNew",
  "import",
  "emptyTitle",
  "emptyDescription",
  "emptyImportHint",
  "dialogTitle",
  "dialogDescription",
  "emptyCatalogDescription",
  "emptyCatalogTitle",
  "detailsAction",
  "selectAction",
  "deselectAction",
  "selectedState",
  "availableState",
  "detailSelectedTitle",
  "detailSelectedDescription",
  "detailAvailableTitle",
  "detailAvailableDescription",
  "useAction",
  "addedAction",
  "teamEmptyState",
]);
function visibleName(name) {
  if (["typeLabel", "kindLabel"].includes(name)) return false;
  return (
    visible.has(name) ||
    /(?:Title|Subtitle|Description|Label|Action|Hint|Placeholder|Message|Caption|Tooltip|Subcopy)$/.test(
      name,
    )
  );
}
// Only these modules return display copy. Protocol/error classification stays literal.
const copyModules = new Set([
  "features/agents/lib/agentCardModelLabel.ts",
  "features/agents/ui/personaLibraryCopy.ts",
  "features/agents/ui/teamLibraryCopy.ts",
  "features/agents/ui/agentUi.ts",
  "features/agents/ui/agentSessionToolSummary.ts",
  "features/notifications/lib/notificationFormat.ts",
  "features/presence/lib/presence.ts",
  "features/sidebar/lib/channelLabels.ts",
  "features/messages/lib/systemEventCopy.ts",
  "features/messages/lib/dateFormatters.ts",
  "features/messages/lib/composerMentionCopy.ts",
  "features/messages/lib/messageLinkLabel.ts",
  "features/workflows/ui/workflowTriggerDescription.ts",
  "features/workflows/ui/workflowStepDescription.ts",
  "features/workflows/ui/workflowActivationWarning.ts",
  "features/settings/lib/appearanceScopeCopy.ts",
  "features/settings/ui/harnessCatalogCopy.ts",
  "features/projects/lib/projectLabels.ts",
]);
const copyFunctions = {
  "features/channels/lib/ephemeralChannel.ts": [
    "formatCompactRemaining",
    "formatVerboseRemaining",
    "formatCompactTtl",
    "formatVerboseTtl",
    "getEphemeralChannelDisplay",
  ],
  "features/home/ui/FeedSection.tsx": ["feedHeadline"],
  "features/messages/ui/DraftsPanel.tsx": ["formatDraftCreatedAt"],
  "features/reminders/ui/RemindersPanel.tsx": [
    "formatRelativeTime",
    "formatReminderSourceLocation",
  ],
  "features/search/ui/SearchResultItem.tsx": ["formatRelativeTime"],
  "features/search/ui/TopbarSearch.tsx": ["formatRelativeTime"],
  "features/agents/lib/formatAgentModelLabel.ts": ["formatAgentModelLabel"],
  "features/agents/lib/managedAgentControlActions.ts": [
    "getManagedAgentPrimaryActionLabel",
  ],
  "features/agents/ui/agentSessionTranscriptGrouping.ts": ["sameKindLabel"],
  "features/agents/ui/bakedEnvHelpers.ts": [
    "getAdvancedInheritedSummary",
    "getBakedModelInheritLabel",
    "getBakedProviderInheritLabel",
  ],
  "features/agents/ui/catalogOwnerLabel.ts": ["resolveCatalogOwnerLabel"],
  "features/agents/ui/personaModelDiscoveryStatus.ts": ["providerObjectLabel"],
  "features/agents/ui/usePersonaModelDiscovery.ts": ["claudeCodeModelLabel"],
  "features/channels/lib/channelDescription.ts": ["getChannelDescription"],
  "features/channels/lib/channelLifecycle.ts": ["channelLifecycleLabel"],
  "features/community-members/lib/joinAlerts.ts": ["joinAlertSummaryBody"],
  "features/home/lib/inbox.ts": [
    "categoryLabelFor",
    "formatInboxTypeLabel",
    "projectTypeLabel",
  ],
  "features/local-archive/ui/localArchiveKinds.ts": ["kindLabel"],
  "features/messages/lib/backgroundMediaUploadPhase.ts": [
    "backgroundMediaUploadPhaseLabel",
  ],
  "features/onboarding/ui/harnessConnectionOptions.ts": [
    "runtimeUnavailableDescription",
  ],
  "features/profile/lib/identity.ts": ["formatOwnerLabel", "resolveUserLabel"],
  "features/projects/lib/projectsViewHelpers.ts": [
    "getActivityLabel",
    "getClonePathLabel",
  ],
  "features/projects/projectTaskCategories.ts": ["projectTaskCategoryLabel"],
  "features/projects/ui/useOpenProjectTerminal.ts": ["projectTerminalLabel"],
  "features/settings/ui/harnessCatalogLogic.ts": [
    "entryStatusLabel",
    "installLinkLabel",
  ],
  "features/sidebar/lib/useSidebarUnreadOverflow.ts": [
    "sidebarOverflowUnreadLabel",
  ],
  "features/workflows/ui/workflowStepDescription.ts": [
    "workflowStepDescription",
  ],
  "features/workflows/ui/workflowDefinition.ts": [
    "getActionCardClause",
    "getScheduleCardClause",
    "getTriggerCardClause",
    "getWorkflowCardLabel",
  ],
};
function copyReturnContext(context, file) {
  if (!context || !ts.isReturnStatement(context)) return false;
  if (copyModules.has(file)) return true;
  let fn = context.parent;
  while (fn && !ts.isFunctionDeclaration(fn)) fn = fn.parent;
  return Boolean(fn && copyFunctions[file]?.includes(fn.name?.getText()));
}
const report = { files: [], untranslated: [], total: 0 };
function all(dir) {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .flatMap((e) =>
      e.isDirectory() ? all(path.join(dir, e.name)) : [path.join(dir, e.name)],
    );
}
function normalize(s) {
  return s.replace(/\s+/g, " ").trim();
}
function displayExpression(n) {
  if (!ts.isJsxExpression(n)) return false;
  return !ts.isJsxAttribute(n.parent) || visibleName(n.parent.name.getText());
}
function jsxText(s) {
  s = s.replace(
    /&(?:amp|lt|gt|quot|apos|nbsp);/g,
    (x) =>
      ({
        "&amp;": "&",
        "&lt;": "<",
        "&gt;": ">",
        "&quot;": '"',
        "&apos;": "'",
        "&nbsp;": "\u00a0",
      })[x],
  );
  const lines = s.split(/\r\n|\n|\r/);
  let last = 0;
  lines.forEach((l, i) => {
    if (/[^ \t]/.test(l)) last = i;
  });
  return lines
    .map((l, i) => {
      l = l.replace(/\t/g, " ");
      if (i) l = l.replace(/^ +/, "");
      if (i < lines.length - 1) l = l.replace(/ +$/, "");
      return l + (l && i !== last ? " " : "");
    })
    .join("");
}
function component(n) {
  if (ts.isFunctionDeclaration(n)) return n.name && /^[A-Z]/.test(n.name.text);
  if (!ts.isArrowFunction(n) && !ts.isFunctionExpression(n)) return false;
  let p = n.parent;
  while (
    p &&
    (ts.isCallExpression(p) ||
      ts.isParenthesizedExpression(p) ||
      ts.isAsExpression(p))
  )
    p = p.parent;
  return (
    p &&
    ts.isVariableDeclaration(p) &&
    ts.isIdentifier(p.name) &&
    /^[A-Z]/.test(p.name.text)
  );
}
function needsSubscription(fn) {
  let hook = false;
  function check(n) {
    if (
      n !== fn.body &&
      (ts.isFunctionDeclaration(n) ||
        ts.isFunctionExpression(n) ||
        ts.isArrowFunction(n))
    )
      return;
    if (
      ts.isCallExpression(n) &&
      /(?:^|\.)use[A-Z]/.test(n.expression.getText())
    )
      hook = true;
    ts.forEachChild(n, check);
  }
  check(fn.body);
  const p = fn.parent;
  const memo =
    ts.isCallExpression(p) && /(?:^|\.)memo$/.test(p.expression.getText());
  return hook || memo;
}
function containsJsx(n) {
  let found = false;
  function visit(c) {
    if (
      c !== n &&
      (ts.isFunctionDeclaration(c) ||
        ts.isFunctionExpression(c) ||
        ts.isArrowFunction(c))
    )
      return;
    if (
      ts.isJsxElement(c) ||
      ts.isJsxSelfClosingElement(c) ||
      ts.isJsxFragment(c)
    ) {
      found = true;
      return;
    }
    ts.forEachChild(c, visit);
  }
  visit(n);
  return found;
}
function displayInitializer(context) {
  if (
    !context ||
    !(
      ts.isVariableDeclaration(context) ||
      ts.isBindingElement(context) ||
      ts.isParameter(context)
    )
  )
    return false;
  const name = context.name.getText();
  if (
    !/^(?:label|title|description|placeholder|hint)$|(?:Label|Title|Description|Placeholder|Hint|Caption|ErrorMessage)$/.test(
      name,
    ) ||
    ["typeLabel", "kindLabel"].includes(name)
  )
    return false;
  let owner = context.parent;
  while (owner && !ts.isFunctionLike(owner)) owner = owner.parent;
  return Boolean(owner);
}
for (const filename of all(root)) {
  const rel = path.relative(root, filename);
  if (
    !/\.tsx?$/.test(filename) ||
    /\.d\.ts$/.test(filename) ||
    /\.(test|jsdom-test)\./.test(filename) ||
    /^(locales|testing|protectedFeatures)[/\\]/.test(rel) ||
    [
      "i18n.ts",
      "language.ts",
      "features/settings/ui/LanguageSetting.tsx",
    ].includes(rel) ||
    rel.startsWith("shared/i18n/")
  )
    continue;
  const text = fs.readFileSync(filename, "utf8");
  const processed = text.includes("// Buzz source localisation:");
  const ast = ts.createSourceFile(
    filename,
    text,
    ts.ScriptTarget.Latest,
    true,
    filename.endsWith("tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const edits = [],
    uses = [],
    functions = [];
  let needsT = false,
    needsMessage = false;
  const replace = (node, value) =>
    edits.push({ start: node.getStart(ast), end: node.end, value });
  function call(source, values, rich = false) {
    const prefix = source.match(/^\s*/)?.[0] ?? "";
    const suffix = source.match(/\s*$/)?.[0] ?? "";
    source = prefix + (known(source) || source.trim()) + suffix;
    if (rich) needsMessage = true;
    else needsT = true;
    uses.push(normalize(source));
    return `${rich ? "uiMessage" : "uiT"}(${JSON.stringify(source)}${values ? `, { ${values.map((v, i) => `p${i}: ${v}`).join(", ")} }` : ""})`;
  }
  function known(source) {
    const s = normalize(source);
    return keys[s] !== undefined ? s : byShape.get(shape(s));
  }
  function skipJsx(n) {
    let p = n.parent;
    while (p) {
      if (ts.isJsxElement(p)) {
        const tag = p.openingElement.tagName.getText(ast);
        if (["code", "pre", "textarea"].includes(tag)) return true;
        if (
          p.openingElement.attributes.properties.some(
            (a) =>
              ts.isJsxAttribute(a) &&
              ["contentEditable", "data-no-translate"].includes(
                a.name.getText(ast),
              ),
          )
        )
          return true;
      }
      p = p.parent;
    }
    return false;
  }
  function visit(n) {
    if (
      !processed &&
      component(n) &&
      n.body &&
      containsJsx(n.body) &&
      needsSubscription(n)
    )
      functions.push(n);
    if (ts.isJsxElement(n) && !skipJsx(n) && n.children.length > 1) {
      const replaced = new Set();
      let group = [];
      function flush() {
        const values = [];
        let source = "",
          rich = false;
        for (const c of group) {
          if (ts.isJsxText(c)) source += jsxText(c.text);
          else if (c.expression) {
            if (ts.isStringLiteralLike(c.expression))
              source += c.expression.text;
            else {
              source += `{${values.length}}`;
              values.push(c.expression.getText(ast));
              function check(x) {
                if (
                  ts.isJsxElement(x) ||
                  ts.isJsxSelfClosingElement(x) ||
                  ts.isJsxFragment(x)
                )
                  rich = true;
                ts.forEachChild(x, check);
              }
              check(c.expression);
            }
          }
        }
        if (values.length && known(source)) {
          edits.push({
            start: group[0].getStart(ast),
            end: group.at(-1).end,
            value: `{${call(normalize(source), values, rich)}}`,
          });
          for (const c of group) replaced.add(c);
        }
        group = [];
      }
      for (const c of n.children) {
        if (ts.isJsxText(c) || ts.isJsxExpression(c)) group.push(c);
        else flush();
      }
      flush();
      if (replaced.size) {
        visit(n.openingElement);
        for (const c of n.children) if (!replaced.has(c)) visit(c);
        return;
      }
    }
    if (ts.isJsxText(n) && !skipJsx(n)) {
      const source = jsxText(n.text);
      if (known(source)) replace(n, `{${call(source)}}`);
      else if (/[A-Za-z]{2}/.test(source))
        report.untranslated.push({
          file: rel,
          line: ast.getLineAndCharacterOfPosition(n.getStart(ast)).line + 1,
          source: normalize(source),
          kind: "text",
        });
      return;
    }
    if (ts.isStringLiteralLike(n)) {
      const p = n.parent;
      const attr = ts.isJsxAttribute(p) && visibleName(p.name.getText(ast));
      const prop =
        ts.isPropertyAssignment(p) &&
        visibleName(
          p.name.getText(ast).replaceAll('"', "").replaceAll("'", ""),
        ) &&
        p.initializer === n;
      const expr = displayExpression(p);
      const branch =
        (ts.isConditionalExpression(p) || ts.isBinaryExpression(p)) &&
        (() => {
          let q = p;
          while (
            q &&
            (ts.isParenthesizedExpression(q) ||
              ts.isConditionalExpression(q) ||
              (ts.isBinaryExpression(q) &&
                [
                  ts.SyntaxKind.QuestionQuestionToken,
                  ts.SyntaxKind.BarBarToken,
                  ts.SyntaxKind.AmpersandAmpersandToken,
                ].includes(q.operatorToken.kind)))
          )
            q = q.parent;
          return q && displayExpression(q);
        })();
      const toast =
        ts.isCallExpression(p) &&
        p.arguments[0] === n &&
        /^toast(?:\.|\()/.test(p.expression.getText(ast));
      let context = p;
      while (
        context &&
        (ts.isConditionalExpression(context) ||
          ts.isParenthesizedExpression(context) ||
          (ts.isBinaryExpression(context) &&
            [
              ts.SyntaxKind.QuestionQuestionToken,
              ts.SyntaxKind.BarBarToken,
            ].includes(context.operatorToken.kind)))
      )
        context = context.parent;
      const copyReturn = copyReturnContext(context, rel);
      const errorCopy =
        context &&
        ts.isCallExpression(context) &&
        /^set(?:\w*Error(?:Message)?|StatusMessage)$/.test(
          context.expression.getText(ast),
        ) &&
        context.arguments[0] &&
        context.arguments[0].pos <= n.pos;
      const toastFallback =
        context &&
        ts.isCallExpression(context) &&
        /^toast\./.test(context.expression.getText(ast)) &&
        context.arguments[0]?.pos <= n.pos &&
        context.arguments[0]?.end >= n.end;
      const copyProperty =
        context &&
        ts.isPropertyAssignment(context) &&
        context.initializer.pos <= n.pos &&
        visibleName(context.name.getText(ast));
      if (
        (attr ||
          prop ||
          expr ||
          branch ||
          toast ||
          copyReturn ||
          errorCopy ||
          toastFallback ||
          displayExpression(context) ||
          displayInitializer(context) ||
          copyProperty) &&
        !skipJsx(n)
      ) {
        if (known(n.text)) {
          const translated = call(n.text);
          if (attr) replace(n, `{${translated}}`);
          else if (prop)
            replace(
              p,
              `get ${p.name.getText(ast)}() { return ${translated}; }`,
            );
          else replace(n, translated);
        } else if (/[A-Za-z]{2}/.test(n.text) && n.text.length > 2)
          report.untranslated.push({
            file: rel,
            line: ast.getLineAndCharacterOfPosition(n.getStart(ast)).line + 1,
            source: n.text,
            kind: prop ? "label" : attr ? "attribute" : "expression",
          });
      }
      return;
    }
    if (ts.isTemplateExpression(n) && !skipJsx(n)) {
      let source = n.head.text;
      const values = [];
      for (const span of n.templateSpans) {
        source += `{${values.length}}${span.literal.text}`;
        values.push(span.expression.getText(ast));
      }
      // Only visible render values / toast arguments. Never translate paths, prompts, or protocol values.
      const p = n.parent;
      const visibleProp =
        ts.isPropertyAssignment(p) && visibleName(p.name.getText(ast));
      let context = p;
      while (
        context &&
        (ts.isConditionalExpression(context) ||
          ts.isParenthesizedExpression(context) ||
          (ts.isBinaryExpression(context) &&
            [
              ts.SyntaxKind.QuestionQuestionToken,
              ts.SyntaxKind.BarBarToken,
            ].includes(context.operatorToken.kind)))
      )
        context = context.parent;
      const render = displayExpression(context) || visibleProp;
      const copyReturn = copyReturnContext(context, rel);
      const errorCopy =
        context &&
        ts.isCallExpression(context) &&
        /^set(?:\w*Error(?:Message)?|StatusMessage)$/.test(
          context.expression.getText(ast),
        );
      const toast =
        ts.isCallExpression(p) &&
        p.arguments[0] === n &&
        p.expression.getText(ast).startsWith("toast");
      if (
        render ||
        toast ||
        copyReturn ||
        errorCopy ||
        displayInitializer(context)
      ) {
        if (known(source)) replace(n, call(source, values));
        else if (/[A-Za-z]{2}/.test(source))
          report.untranslated.push({
            file: rel,
            line: ast.getLineAndCharacterOfPosition(n.getStart(ast)).line + 1,
            source,
            kind: "template",
          });
      }
      return;
    }
    ts.forEachChild(n, visit);
  }
  visit(ast);
  // Every function component observes language changes, including components reading translated label getters.
  for (const fn of functions) {
    const body = fn.body;
    let hasMemo = false;
    const memoComments = new Set();
    function memo(n) {
      if (n !== body && component(n)) return;
      if (
        ts.isCallExpression(n) &&
        /(?:^|\.)useMemo$/.test(n.expression.getText(ast)) &&
        n.arguments[1] &&
        ts.isArrayLiteralExpression(n.arguments[1]) &&
        /uiT\(|\.label\b|\.title\b|\.description\b/.test(
          n.arguments[0].getText(ast),
        )
      ) {
        const a = n.arguments[1];
        edits.push({
          start: a.end - 1,
          end: a.end - 1,
          value: `${a.elements.length && !a.elements.hasTrailingComma ? ", " : ""}__buzzLanguage`,
        });
        hasMemo = true;
        let statement = n;
        while (
          statement.parent &&
          !ts.isVariableStatement(statement) &&
          !ts.isReturnStatement(statement) &&
          !ts.isExpressionStatement(statement)
        )
          statement = statement.parent;
        const pos = statement.getStart(ast);
        if (!memoComments.has(pos)) {
          memoComments.add(pos);
          edits.push({
            start: pos,
            end: pos,
            value:
              "// biome-ignore lint/correctness/useExhaustiveDependencies: UI label getters depend on the active language.\n",
          });
        }
      }
      ts.forEachChild(n, memo);
    }
    memo(body);
    const hook = hasMemo
      ? "const __buzzLanguage = useUiLanguage();"
      : "useUiLanguage();";
    if (ts.isBlock(body))
      edits.push({
        start: body.getStart(ast) + 1,
        end: body.getStart(ast) + 1,
        value: `\n  ${hook}\n`,
      });
    else {
      edits.push({
        start: body.getStart(ast),
        end: body.getStart(ast),
        value: `{ ${hook} return (`,
      });
      edits.push({ start: body.end, end: body.end, value: "); }" });
    }
  }
  if (!edits.length) continue;
  const imports = [
    ...(needsMessage ? ["uiMessage"] : []),
    ...(needsT ? ["uiT"] : []),
    ...(functions.length ? ["useUiLanguage"] : []),
  ].filter(
    (name) =>
      !new RegExp(
        `import \\{[^}]*\\b${name}\\b[^}]*\\} from ["']@/shared/i18n/uiTranslation["']`,
      ).test(text),
  );
  // Reject overlapping replacements instead of silently producing malformed source.
  const replacements = edits
    .filter((e) => e.start !== e.end)
    .sort((a, b) => a.start - b.start);
  for (let i = 1; i < replacements.length; i++)
    if (replacements[i].start < replacements[i - 1].end)
      throw new Error(`Overlapping localisation edits in ${rel}`);
  let next = text;
  for (const e of edits.sort((a, b) => b.start - a.start || b.end - a.end))
    next = next.slice(0, e.start) + e.value + next.slice(e.end);
  next =
    (processed
      ? ""
      : "// Buzz source localisation: authored UI strings only; user values stay unchanged.\n") +
    (imports.length
      ? `import { ${imports.join(", ")} } from "@/shared/i18n/uiTranslation";\n`
      : "") +
    next;
  if (write) fs.writeFileSync(filename, next);
  report.files.push({
    file: rel,
    messages: uses.length,
    components: functions.length,
  });
  report.total += uses.length;
}
const reportIndex = process.argv.indexOf("--report");
if (reportIndex >= 0)
  fs.writeFileSync(
    process.argv[reportIndex + 1],
    JSON.stringify(report, null, 2) + "\n",
  );
console.log(
  JSON.stringify({
    write,
    files: report.files.length,
    translatedCallSites: report.total,
    untranslatedCandidates: report.untranslated.length,
  }),
);
