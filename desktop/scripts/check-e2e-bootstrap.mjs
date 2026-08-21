import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const SPEC_PATTERN = /\.(spec|perf)\.ts$/;
const TEST_MEMBERS = new Set(["only", "skip", "fixme", "fail"]);
const CONTAINERS = new Set(["forEach", "map"]);

function callbackOf(call) {
  const callback = call.arguments.at(-1);
  return callback &&
    (ts.isArrowFunction(callback) || ts.isFunctionExpression(callback))
    ? callback
    : undefined;
}

function bindingContainsPageFixture(name) {
  if (ts.isIdentifier(name)) return name.text === "page";
  if (ts.isObjectBindingPattern(name))
    return name.elements.some(
      (element) =>
        element.propertyName?.getText() === "page" ||
        (!element.propertyName && bindingContainsPageFixture(element.name)),
    );
  return false;
}

function stringKey(expression) {
  return ts.isStringLiteral(expression) ||
    ts.isNoSubstitutionTemplateLiteral(expression)
    ? expression.text
    : undefined;
}

function bindingInitializers(callback) {
  const bindings = [];
  const collect = (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.initializer
    )
      bindings.push([node.name.text, node.initializer]);
    if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind === ts.SyntaxKind.EqualsToken &&
      ts.isIdentifier(node.left)
    )
      bindings.push([node.left.text, node.right]);
    ts.forEachChild(node, collect);
  };
  collect(callback.body);
  return bindings;
}

function callbackUsesPageFixture(callback) {
  if (
    callback.parameters.some((parameter) =>
      bindingContainsPageFixture(parameter.name),
    )
  )
    return true;

  const fixtureObjects = new Set();
  for (const parameter of callback.parameters) {
    if (ts.isIdentifier(parameter.name))
      fixtureObjects.add(parameter.name.text);
    if (ts.isObjectBindingPattern(parameter.name)) {
      for (const element of parameter.name.elements) {
        if (element.dotDotDotToken && ts.isIdentifier(element.name))
          fixtureObjects.add(element.name.text);
      }
    }
  }

  const bindings = bindingInitializers(callback);
  let changed = true;
  while (changed) {
    changed = false;
    for (const [name, initializer] of bindings) {
      if (
        ts.isIdentifier(initializer) &&
        fixtureObjects.has(initializer.text) &&
        !fixtureObjects.has(name)
      ) {
        fixtureObjects.add(name);
        changed = true;
      }
    }
  }

  let usesPage = false;
  const visit = (node) => {
    if (usesPage) return;
    if (
      ts.isPropertyAccessExpression(node) &&
      ts.isIdentifier(node.expression) &&
      fixtureObjects.has(node.expression.text) &&
      node.name.text === "page"
    ) {
      usesPage = true;
      return;
    }
    if (
      ts.isElementAccessExpression(node) &&
      ts.isIdentifier(node.expression) &&
      fixtureObjects.has(node.expression.text) &&
      node.argumentExpression &&
      stringKey(node.argumentExpression) === "page"
    ) {
      usesPage = true;
      return;
    }
    if (
      ts.isVariableDeclaration(node) &&
      ts.isObjectBindingPattern(node.name) &&
      node.initializer &&
      ts.isIdentifier(node.initializer) &&
      fixtureObjects.has(node.initializer.text) &&
      bindingContainsPageFixture(node.name)
    ) {
      usesPage = true;
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(callback.body);
  return usesPage || callbackCreatesPage(callback);
}

function fixturePageReceivers(callback) {
  const receivers = new Set();
  const fixtureObjects = new Set();
  for (const parameter of callback.parameters) {
    if (ts.isIdentifier(parameter.name))
      fixtureObjects.add(parameter.name.text);
    if (ts.isObjectBindingPattern(parameter.name)) {
      for (const element of parameter.name.elements) {
        if (element.dotDotDotToken && ts.isIdentifier(element.name))
          fixtureObjects.add(element.name.text);
        if (
          ts.isIdentifier(element.name) &&
          (element.propertyName?.getText() === "page" ||
            (!element.propertyName && element.name.text === "page"))
        )
          receivers.add(element.name.text);
      }
    }
  }
  const bindings = bindingInitializers(callback);
  let changed = true;
  while (changed) {
    changed = false;
    for (const [name, initializer] of bindings) {
      if (
        ts.isIdentifier(initializer) &&
        fixtureObjects.has(initializer.text) &&
        !fixtureObjects.has(name)
      ) {
        fixtureObjects.add(name);
        changed = true;
      }
    }
  }
  const visit = (node) => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isObjectBindingPattern(node.name) &&
      node.initializer &&
      ts.isIdentifier(node.initializer) &&
      fixtureObjects.has(node.initializer.text)
    ) {
      for (const element of node.name.elements) {
        if (
          ts.isIdentifier(element.name) &&
          (element.propertyName?.getText() === "page" ||
            (!element.propertyName && element.name.text === "page"))
        )
          receivers.add(element.name.text);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(callback.body);
  return receivers;
}

function memberExpression(expression) {
  const member = unwrapParentheses(expression);
  return ts.isPropertyAccessExpression(member) ||
    ts.isElementAccessExpression(member)
    ? member
    : undefined;
}

function propertyName(expression) {
  const member = memberExpression(expression);
  if (!member) return undefined;
  if (ts.isPropertyAccessExpression(member)) return member.name.text;
  return member.argumentExpression &&
    ts.isStringLiteral(member.argumentExpression)
    ? member.argumentExpression.text
    : undefined;
}

function receiverName(expression) {
  const member = memberExpression(expression);
  if (!member) return undefined;
  const receiver = unwrapParentheses(member.expression);
  return ts.isIdentifier(receiver) ? receiver.text : undefined;
}

function createdPageReceiver(initializer) {
  let expression = unwrapParentheses(initializer);
  if (ts.isAwaitExpression(expression))
    expression = unwrapParentheses(expression.expression);
  return (
    ts.isCallExpression(expression) &&
    propertyName(expression.expression) === "newPage"
  );
}

function createdPageAliases(callback) {
  const createdPages = new Set();
  const bindings = bindingInitializers(callback);
  let changed = true;
  while (changed) {
    changed = false;
    for (const [name, initializer] of bindings) {
      const isCreated = createdPageReceiver(initializer);
      const isAlias =
        ts.isIdentifier(initializer) && createdPages.has(initializer.text);
      if ((isCreated || isAlias) && !createdPages.has(name)) {
        createdPages.add(name);
        changed = true;
      }
    }
  }
  return createdPages;
}

function callbackCreatesPage(callback) {
  return createdPageAliases(callback).size > 0;
}

function directCall(statement) {
  const expression =
    ts.isExpressionStatement(statement) || ts.isReturnStatement(statement)
      ? statement.expression
      : ts.isVariableStatement(statement) &&
          statement.declarationList.declarations.length === 1
        ? statement.declarationList.declarations[0].initializer
        : undefined;
  let unwrapped = expression ? unwrapParentheses(expression) : undefined;
  if (unwrapped && ts.isAwaitExpression(unwrapped))
    unwrapped = unwrapParentheses(unwrapped.expression);
  return unwrapped && ts.isCallExpression(unwrapped) ? unwrapped : undefined;
}

function calleeIdentifier(call) {
  const callee = unwrapParentheses(call.expression);
  return ts.isIdentifier(callee) ? callee : undefined;
}

function readsLocalStorage(node) {
  let found = false;
  const visit = (child) => {
    if (ts.isIdentifier(child) && child.text === "localStorage") found = true;
    if (!found) ts.forEachChild(child, visit);
  };
  ts.forEachChild(node, visit);
  return found;
}

function unsafeOperationReceiver(call) {
  const receiver = receiverName(call.expression);
  if (!receiver) return undefined;
  const operation = propertyName(call.expression);
  if (
    operation !== "goto" &&
    !(operation === "evaluate" && call.arguments.some(readsLocalStorage))
  )
    return undefined;
  return receiver;
}

function callbackEvents(callback, canonical, helpers, seen = new Set()) {
  const parameters = callback.parameters.map((parameter) =>
    ts.isIdentifier(parameter.name) ? parameter.name.text : undefined,
  );
  const events = [];
  for (const { statement, conditional } of executableStatements(callback)) {
    const call = directCall(statement);
    if (!call) continue;
    const awaited = awaitedCall(statement);
    if (
      !conditional &&
      awaited &&
      calleeIdentifier(awaited)?.text === canonical.bootstrap &&
      awaited.arguments[0] &&
      ts.isIdentifier(awaited.arguments[0])
    ) {
      const index = parameters.indexOf(awaited.arguments[0].text);
      if (index >= 0) events.push({ kind: "bootstrap", index });
    }
    const receiver = unsafeOperationReceiver(call);
    if (receiver) {
      const index = parameters.indexOf(receiver);
      if (index < 0) return undefined;
      events.push({ kind: "unsafe", index });
    }
    const helperName = calleeIdentifier(call)?.text;
    if (helperName) {
      const helper = helpers.get(helperName);
      if (!helper) continue;
      if (seen.has(helperName)) return undefined;
      const nestedSeen = new Set(seen).add(helperName);
      const nestedEvents = callbackEvents(
        helper,
        canonical,
        helpers,
        nestedSeen,
      );
      if (!nestedEvents) return undefined;
      for (const event of nestedEvents) {
        const argument = call.arguments[event.index];
        if (!argument || !ts.isIdentifier(argument)) return undefined;
        const index = parameters.indexOf(argument.text);
        if (index < 0) return undefined;
        events.push({
          kind:
            event.kind === "bootstrap" && (!awaited || conditional)
              ? "conditional-bootstrap"
              : event.kind,
          index,
        });
      }
    }
  }
  return events;
}

function hasUnbootstrappedReceiverOperation(
  callback,
  canonical,
  helpers,
  fixtureReceiversBootstrapped = false,
) {
  const receivers = new Set([
    ...createdPageAliases(callback),
    ...fixturePageReceivers(callback),
  ]);
  if (!receivers.size) return false;
  const bootstrapped = fixtureReceiversBootstrapped
    ? fixturePageReceivers(callback)
    : new Set();
  for (const { statement, conditional } of executableStatements(callback)) {
    const call = directCall(statement);
    if (!call) continue;
    const awaited = awaitedCall(statement);
    if (
      !conditional &&
      awaited &&
      calleeIdentifier(awaited)?.text === canonical.bootstrap &&
      awaited.arguments[0] &&
      ts.isIdentifier(awaited.arguments[0]) &&
      receivers.has(awaited.arguments[0].text)
    )
      bootstrapped.add(awaited.arguments[0].text);
    const receiver = unsafeOperationReceiver(call);
    if (receiver && receivers.has(receiver) && !bootstrapped.has(receiver))
      return true;
    const helperName = calleeIdentifier(call)?.text;
    if (helperName) {
      const helper = helpers.get(helperName);
      if (!helper) continue;
      const events = callbackEvents(
        helper,
        canonical,
        helpers,
        new Set([helperName]),
      );
      if (!events) return true;
      for (const event of events) {
        const argument = call.arguments[event.index];
        if (!argument || !ts.isIdentifier(argument)) return true;
        const mappedReceiver = argument.text;
        if (!receivers.has(mappedReceiver)) return true;
        if (event.kind === "unsafe" && !bootstrapped.has(mappedReceiver))
          return true;
        if (!conditional && event.kind === "bootstrap")
          bootstrapped.add(mappedReceiver);
      }
    }
  }
  return false;
}

function memberOf(call) {
  return ts.isPropertyAccessExpression(call.expression) &&
    ts.isIdentifier(call.expression.expression) &&
    call.expression.expression.text === "test"
    ? call.expression.name.text
    : undefined;
}

function helperImport(file, filename, canonicalHelperPath) {
  for (const statement of file.statements) {
    if (
      !ts.isImportDeclaration(statement) ||
      !ts.isStringLiteral(statement.moduleSpecifier)
    )
      continue;
    const resolved = path.resolve(
      path.dirname(filename),
      `${statement.moduleSpecifier.text}.ts`,
    );
    const canonical = canonicalHelperPath
      ? resolved === canonicalHelperPath
      : statement.moduleSpecifier.text === "../helpers/test";
    if (!canonical) continue;
    const elements = statement.importClause?.namedBindings;
    if (!elements || !ts.isNamedImports(elements)) return undefined;
    const names = new Map(
      elements.elements.map((element) => [
        element.name.text,
        element.propertyName?.text ?? element.name.text,
      ]),
    );
    if (
      names.get("test") === "test" &&
      names.get("bootstrapE2ePage") === "bootstrapE2ePage"
    )
      return { test: "test", bootstrap: "bootstrapE2ePage" };
  }
}

function bindingNameShadows(name, canonical) {
  if (ts.isIdentifier(name))
    return name.text === canonical.test || name.text === canonical.bootstrap;
  return name.elements.some(
    (element) =>
      !ts.isOmittedExpression(element) &&
      bindingNameShadows(element.name, canonical),
  );
}

function hasShadowingDeclaration(file, canonical) {
  let shadowed = false;
  const visit = (node) => {
    if (shadowed) return;
    if (
      ts.isVariableDeclaration(node) &&
      bindingNameShadows(node.name, canonical)
    ) {
      shadowed = true;
      return;
    }
    if (
      (ts.isFunctionDeclaration(node) || ts.isClassDeclaration(node)) &&
      node.name &&
      ts.isIdentifier(node.name) &&
      (node.name.text === canonical.test ||
        node.name.text === canonical.bootstrap)
    ) {
      shadowed = true;
      return;
    }
    if (
      (ts.isArrowFunction(node) ||
        ts.isFunctionExpression(node) ||
        ts.isFunctionDeclaration(node) ||
        ts.isMethodDeclaration(node)) &&
      node.parameters.some((parameter) =>
        bindingNameShadows(parameter.name, canonical),
      )
    ) {
      shadowed = true;
      return;
    }
    ts.forEachChild(node, visit);
  };
  for (const statement of file.statements) {
    if (ts.isImportDeclaration(statement)) continue;
    visit(statement);
  }
  return shadowed;
}

function collectModuleHelpers(file) {
  const helpers = new Map();
  for (const statement of file.statements) {
    if (ts.isFunctionDeclaration(statement) && statement.name && statement.body)
      helpers.set(statement.name.text, statement);
    if (ts.isVariableStatement(statement)) {
      for (const declaration of statement.declarationList.declarations) {
        if (
          ts.isIdentifier(declaration.name) &&
          declaration.initializer &&
          (ts.isArrowFunction(declaration.initializer) ||
            ts.isFunctionExpression(declaration.initializer))
        )
          helpers.set(declaration.name.text, declaration.initializer);
      }
    }
  }
  return helpers;
}

function awaitedCall(statement) {
  const expression =
    ts.isExpressionStatement(statement) || ts.isReturnStatement(statement)
      ? statement.expression
      : ts.isVariableStatement(statement) &&
          statement.declarationList.declarations.length === 1
        ? statement.declarationList.declarations[0].initializer
        : undefined;
  const unwrapped = expression ? unwrapParentheses(expression) : undefined;
  if (!unwrapped || !ts.isAwaitExpression(unwrapped)) return undefined;
  const call = unwrapParentheses(unwrapped.expression);
  return ts.isCallExpression(call) ? call : undefined;
}

function unwrapParentheses(expression) {
  while (ts.isParenthesizedExpression(expression))
    expression = expression.expression;
  return expression;
}

function isStaticallyEmptyLoop(statement) {
  if (ts.isForStatement(statement)) {
    const condition = statement.condition
      ? unwrapParentheses(statement.condition)
      : undefined;
    return condition?.kind === ts.SyntaxKind.FalseKeyword;
  }
  if (ts.isWhileStatement(statement))
    return (
      unwrapParentheses(statement.expression).kind ===
      ts.SyntaxKind.FalseKeyword
    );
  if (ts.isForOfStatement(statement)) {
    const expression = guaranteedIterableExpression(statement);
    return (
      (ts.isArrayLiteralExpression(expression) &&
        !expression.elements.length) ||
      (ts.isStringLiteral(expression) && !expression.text.length)
    );
  }
  if (ts.isForInStatement(statement)) {
    const expression = unwrapParentheses(statement.expression);
    return (
      ts.isObjectLiteralExpression(expression) && !expression.properties.length
    );
  }
  return false;
}

function unwrapConstAssertion(expression) {
  expression = unwrapParentheses(expression);
  return ts.isAsExpression(expression) && expression.type.getText() === "const"
    ? unwrapParentheses(expression.expression)
    : expression;
}

function numericLiteral(expression) {
  expression = unwrapParentheses(expression);
  return ts.isNumericLiteral(expression) ? Number(expression.text) : undefined;
}

function staticallyTrueComparison(expression, bindings = new Map()) {
  expression = unwrapParentheses(expression);
  if (!ts.isBinaryExpression(expression)) return false;
  const left = ts.isIdentifier(expression.left)
    ? bindings.get(expression.left.text)
    : numericLiteral(expression.left);
  const right = ts.isIdentifier(expression.right)
    ? bindings.get(expression.right.text)
    : numericLiteral(expression.right);
  if (left === undefined || right === undefined) return false;
  switch (expression.operatorToken.kind) {
    case ts.SyntaxKind.LessThanToken:
      return left < right;
    case ts.SyntaxKind.LessThanEqualsToken:
      return left <= right;
    case ts.SyntaxKind.GreaterThanToken:
      return left > right;
    case ts.SyntaxKind.GreaterThanEqualsToken:
      return left >= right;
    case ts.SyntaxKind.EqualsEqualsToken:
    case ts.SyntaxKind.EqualsEqualsEqualsToken:
      return left === right;
    default:
      return false;
  }
}

function precedingConstInitializer(statement, name) {
  const parent = statement.parent;
  if (!ts.isBlock(parent)) return undefined;
  for (const sibling of parent.statements) {
    if (sibling === statement) return undefined;
    if (!ts.isVariableStatement(sibling)) continue;
    if (!(sibling.declarationList.flags & ts.NodeFlags.Const)) continue;
    for (const declaration of sibling.declarationList.declarations)
      if (
        ts.isIdentifier(declaration.name) &&
        declaration.name.text === name &&
        declaration.initializer
      )
        return declaration.initializer;
  }
  return undefined;
}

function guaranteedIterableExpression(statement) {
  const expression = unwrapConstAssertion(statement.expression);
  if (!ts.isIdentifier(expression)) return expression;
  const initializer = precedingConstInitializer(statement, expression.text);
  return initializer ? unwrapConstAssertion(initializer) : expression;
}

function isConcreteObjectContributor(property) {
  if (ts.isSpreadAssignment(property)) return false;
  if (ts.isComputedPropertyName(property.name)) {
    const expression = unwrapParentheses(property.name.expression);
    return (
      ts.isStringLiteral(expression) ||
      ts.isNoSubstitutionTemplateLiteral(expression) ||
      ts.isNumericLiteral(expression)
    );
  }
  return (
    !ts.isPropertyAssignment(property) || property.name.text !== "__proto__"
  );
}

function isGuaranteedIteration(statement) {
  if (ts.isDoStatement(statement)) return true;
  if (ts.isWhileStatement(statement))
    return isStaticallyTrue(statement.expression);
  if (ts.isForOfStatement(statement)) {
    const expression = guaranteedIterableExpression(statement);
    return (
      (ts.isArrayLiteralExpression(expression) &&
        expression.elements.some((element) => !ts.isSpreadElement(element))) ||
      (ts.isStringLiteral(expression) && expression.text.length > 0)
    );
  }
  if (ts.isForInStatement(statement)) {
    const expression = unwrapParentheses(statement.expression);
    return (
      ts.isObjectLiteralExpression(expression) &&
      expression.properties.some(isConcreteObjectContributor)
    );
  }
  if (!ts.isForStatement(statement)) return false;
  if (!statement.condition || isStaticallyTrue(statement.condition))
    return true;
  const numericBindings = new Map();
  if (
    statement.initializer &&
    ts.isVariableDeclarationList(statement.initializer)
  )
    for (const declaration of statement.initializer.declarations) {
      const value = declaration.initializer
        ? numericLiteral(declaration.initializer)
        : undefined;
      if (ts.isIdentifier(declaration.name) && value !== undefined)
        numericBindings.set(declaration.name.text, value);
    }
  return staticallyTrueComparison(statement.condition, numericBindings);
}

function isStaticallyFalse(expression) {
  return unwrapParentheses(expression).kind === ts.SyntaxKind.FalseKeyword;
}

function isStaticallyTrue(expression) {
  return unwrapParentheses(expression).kind === ts.SyntaxKind.TrueKeyword;
}

function* executableStatements(callback) {
  const statements = ts.isBlock(callback.body)
    ? callback.body.statements
    : [ts.factory.createExpressionStatement(callback.body)];
  function* walk(statement, conditional = false) {
    if (ts.isBlock(statement)) {
      for (const child of statement.statements) yield* walk(child, conditional);
      return;
    }
    if (ts.isIfStatement(statement)) {
      if (!isStaticallyFalse(statement.expression))
        yield* walk(statement.thenStatement, true);
      if (statement.elseStatement && !isStaticallyTrue(statement.expression))
        yield* walk(statement.elseStatement, true);
      return;
    }
    if (ts.isSwitchStatement(statement)) {
      for (const clause of statement.caseBlock.clauses)
        for (const child of clause.statements) yield* walk(child, true);
      return;
    }
    if (ts.isTryStatement(statement)) {
      yield* walk(statement.tryBlock, conditional);
      if (statement.catchClause) yield* walk(statement.catchClause.block, true);
      if (statement.finallyBlock)
        yield* walk(statement.finallyBlock, conditional);
      return;
    }
    if (
      (ts.isForStatement(statement) ||
        ts.isForInStatement(statement) ||
        ts.isForOfStatement(statement) ||
        ts.isWhileStatement(statement) ||
        ts.isDoStatement(statement)) &&
      !isStaticallyEmptyLoop(statement)
    ) {
      yield* walk(
        statement.statement,
        conditional || !isGuaranteedIteration(statement),
      );
      return;
    }
    yield { statement, conditional };
  }
  for (const statement of statements) yield* walk(statement);
}

function bootstrapReceivers(
  callback,
  canonical,
  helpers,
  fixtureReceiversBootstrapped = false,
) {
  const receivers = fixtureReceiversBootstrapped
    ? fixturePageReceivers(callback)
    : new Set();
  for (const { statement, conditional } of executableStatements(callback)) {
    const call = awaitedCall(statement);
    if (!call) continue;
    if (
      !conditional &&
      calleeIdentifier(call)?.text === canonical.bootstrap &&
      call.arguments[0] &&
      ts.isIdentifier(call.arguments[0])
    )
      receivers.add(call.arguments[0].text);
    const helperName = calleeIdentifier(call)?.text;
    if (!conditional && helperName) {
      const helper = helpers.get(helperName);
      if (helper) {
        const events = callbackEvents(
          helper,
          canonical,
          helpers,
          new Set([helperName]),
        );
        if (!events) return undefined;
        for (const event of events) {
          if (event.kind !== "bootstrap") continue;
          const argument = call.arguments[event.index];
          if (argument && ts.isIdentifier(argument))
            receivers.add(argument.text);
        }
      }
    }
  }
  return receivers;
}

function callbackBootstraps(
  callback,
  canonical,
  helpers,
  fixtureReceiversBootstrapped = false,
) {
  if (
    hasUnbootstrappedReceiverOperation(
      callback,
      canonical,
      helpers,
      fixtureReceiversBootstrapped,
    )
  )
    return false;
  const receivers = bootstrapReceivers(
    callback,
    canonical,
    helpers,
    fixtureReceiversBootstrapped,
  );
  if (!receivers) return false;
  return (
    [...fixturePageReceivers(callback)].every((receiver) =>
      receivers.has(receiver),
    ) && receivers.size > 0
  );
}

function makeScope(parent) {
  return { parent, hooks: [], tests: [], children: [] };
}

function hasPageFixtureTest(file) {
  let found = false;
  const visit = (node) => {
    if (found) return;
    if (ts.isCallExpression(node)) {
      const callback = callbackOf(node);
      if (
        callback &&
        ((ts.isIdentifier(node.expression) &&
          node.expression.text === "test") ||
          (ts.isPropertyAccessExpression(node.expression) &&
            ts.isIdentifier(node.expression.expression) &&
            node.expression.expression.text === "test" &&
            (TEST_MEMBERS.has(node.expression.name.text) ||
              node.expression.name.text === "beforeEach"))) &&
        callbackUsesPageFixture(callback)
      ) {
        found = true;
        return;
      }
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(file, visit);
  return found;
}

function scanFile(file, canonical) {
  const helpers = collectModuleHelpers(file);
  const root = makeScope(undefined);
  const isTestCall = (call) =>
    ts.isIdentifier(call.expression) && call.expression.text === canonical.test;
  const isMember = (call, name) => memberOf(call) === name;

  const scanStatements = (statements, scope) => {
    for (const statement of statements) {
      if (ts.isBlock(statement)) {
        scanStatements(statement.statements, scope);
        continue;
      }
      if (
        ts.isForStatement(statement) ||
        ts.isForInStatement(statement) ||
        ts.isForOfStatement(statement)
      ) {
        scanStatements(
          ts.isBlock(statement.statement)
            ? statement.statement.statements
            : [statement.statement],
          scope,
        );
        continue;
      }
      if (
        !ts.isExpressionStatement(statement) ||
        !ts.isCallExpression(statement.expression)
      )
        continue;
      const call = statement.expression;
      const callback = callbackOf(call);
      if (isMember(call, "describe") && callback) {
        const child = makeScope(scope);
        scope.children.push(child);
        if (ts.isBlock(callback.body))
          scanStatements(callback.body.statements, child);
        continue;
      }
      if (isMember(call, "beforeEach") && callback) {
        scope.hooks.push(callbackBootstraps(callback, canonical, helpers));
        continue;
      }
      if (
        isTestCall(call) ||
        (memberOf(call) && TEST_MEMBERS.has(memberOf(call)))
      ) {
        if (callback)
          scope.tests.push({
            bootstraps: callbackBootstraps(callback, canonical, helpers),
            bootstrapsWithFixtureHook: callbackBootstraps(
              callback,
              canonical,
              helpers,
              true,
            ),
            usesPage: callbackUsesPageFixture(callback),
          });
        continue;
      }
      if (
        ts.isPropertyAccessExpression(call.expression) &&
        CONTAINERS.has(call.expression.name.text) &&
        callback &&
        ts.isArrayLiteralExpression(call.expression.expression)
      ) {
        if (ts.isBlock(callback.body))
          scanStatements(callback.body.statements, scope);
      }
    }
  };
  scanStatements(file.statements, root);
  return root;
}

function everyTestCovered(scope, inheritedHook = false) {
  const coveredByHook = inheritedHook || scope.hooks.some(Boolean);
  return (
    scope.tests.every(
      (test) =>
        !test.usesPage ||
        test.bootstraps ||
        (coveredByHook && test.bootstrapsWithFixtureHook),
    ) && scope.children.every((child) => everyTestCovered(child, coveredByHook))
  );
}

function testCount(scope) {
  return (
    scope.tests.filter((test) => test.usesPage).length +
    scope.children.reduce((count, child) => count + testCount(child), 0)
  );
}

export function checkSource(
  source,
  filename = "fixture.spec.ts",
  canonicalHelperPath,
) {
  const file = ts.createSourceFile(
    filename,
    source,
    ts.ScriptTarget.Latest,
    true,
  );
  const canonical = helperImport(file, filename, canonicalHelperPath);
  if (!canonical)
    return "must directly import unaliased test and bootstrapE2ePage from the canonical tests/helpers/test module";
  if (hasShadowingDeclaration(file, canonical))
    return "must not shadow canonical test or bootstrapE2ePage imports";
  const scope = scanFile(file, canonical);
  if (!testCount(scope)) return "does not register a browser test";
  if (!everyTestCovered(scope))
    return "must structurally await bootstrapE2ePage for every test, directly or through an applicable ancestor test.beforeEach";
}

function specFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const child = path.join(directory, entry.name);
    if (entry.isDirectory()) return specFiles(child);
    return entry.isFile() && SPEC_PATTERN.test(entry.name) ? [child] : [];
  });
}

export function runCheck(projectRoot) {
  const e2eRoot = path.join(projectRoot, "tests/e2e");
  const canonicalHelperPath = path.join(projectRoot, "tests/helpers/test.ts");
  return specFiles(e2eRoot).flatMap((filename) => {
    const relative = path.relative(e2eRoot, filename);
    const source = fs.readFileSync(filename, "utf8");
    const file = ts.createSourceFile(
      filename,
      source,
      ts.ScriptTarget.Latest,
      true,
    );
    if (!hasPageFixtureTest(file)) return [];
    const violation = checkSource(source, filename, canonicalHelperPath);
    return violation ? [`${relative}: ${violation}`] : [];
  });
}

if (process.argv[1] === import.meta.filename) {
  const violations = runCheck(path.resolve(import.meta.dirname, ".."));
  if (violations.length) {
    console.error(
      `E2E specs must register bootstrapE2ePage after their setup:\n${violations.join("\n")}`,
    );
    process.exit(1);
  }
}
