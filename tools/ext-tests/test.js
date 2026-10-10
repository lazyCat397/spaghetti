'use strict';
const assert = require('assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const vscode = require('vscode');
const m = global.__vscode;

const SPG = process.env.SPG_BIN;
const EXT = process.env.EXT_JS;
let n = 0;
function mkdoc(text) {
  const file = path.join(os.tmpdir(), `exttest_${process.pid}_${n++}.spg`);
  fs.writeFileSync(file, text);
  return { uri: vscode.Uri.file(file), languageId: 'spg', version: 1, isDirty: false, getText: () => text, _file: file };
}
const tests = [];
const test = (name, fn) => tests.push([name, fn]);

async function complete(text, col, line = 0) {
  const doc = mkdoc(text);
  m.registered.handlers.open(doc);
  const items = await m.registered.completion.provider.provideCompletionItems(doc, new vscode.Position(line, col));
  fs.unlinkSync(doc._file);
  return items;
}
const labels = (items) => items.map((i) => i.label);

test("provider is registered for 'spg' with '.' and '@' as trigger characters", () => {
  assert.strictEqual(m.registered.completion.lang, 'spg');
  assert.deepStrictEqual(m.registered.completion.triggers, ['.', '@']);
});

test("typing '.' after a namespace lists only its members", async () => {
  const items = await complete('let a = @math.\n', 14);
  for (const w of ['sqrt', 'abs', 'random', 'pi', 'e']) assert.ok(labels(items).includes(w), 'missing ' + w);
  assert.ok(!labels(items).some((l) => l === 'print' || l === 'now' || l === 'while'), 'leaked non-math items');
  assert.ok(items.every((i) => i.insertText === undefined), 'members insert their own label');
});

test("typing '@' offers exactly the five namespaces", async () => {
  const items = await complete('let a = @\n', 9);
  assert.deepStrictEqual(labels(items).sort(), ['@array', '@into', '@math', '@str', '@time']);
});

test("a namespace picked from the list replaces '@ma' with '@math.' and re-triggers", async () => {
  const items = await complete('let a = @ma\n', 11);
  const math = items.find((i) => i.label === '@math');
  assert.ok(math, 'no @math entry');
  assert.strictEqual(math.kind, vscode.CompletionItemKind.Module);
  assert.strictEqual(math.insertText, '@math.');
  assert.deepStrictEqual([math.range.start.character, math.range.end.character], [8, 11]); // the '@' is replaced too
  assert.strictEqual(math.command.command, 'editor.action.triggerSuggest');
  const plain = await complete('let a = pr\n', 10);
  const print = plain.find((i) => i.label === 'print');
  assert.ok(print.insertText === undefined && print.command === undefined, 'ordinary entries stay plain');
});

test("full flow: pick '@time' -> text becomes '@time.' -> reopened list shows its members", async () => {
  const before = 'let t = @ti';
  const items = await complete(before + '\n', before.length);
  const pick = items.find((i) => i.label === '@time');
  assert.ok(pick, 'no @time namespace entry');
  const r = pick.range;                                         // apply the edit the way the editor would
  const after = before.slice(0, r.start.character) + pick.insertText + before.slice(r.end.character);
  assert.strictEqual(after, 'let t = @time.');
  assert.strictEqual(pick.command.command, 'editor.action.triggerSuggest'); // the editor re-runs completion here
  const members = await complete(after + '\n', after.length);
  for (const w of ['now', 'unix', 'elapsed', 'sleep']) assert.ok(labels(members).includes(w), 'missing ' + w);
  assert.ok(!labels(members).includes('sqrt') && !labels(members).includes('run'));
});

test("'.' after something that is not a namespace offers nothing", async () => {
  for (const [text, col] of [['let a = p.\n', 10], ['let a = 1.\n', 10], ['let a = f(x).\n', 14], ['let a = math.\n', 13]]) {
    const items = await complete(text, col);
    assert.deepStrictEqual(labels(items), [], 'expected nothing for ' + JSON.stringify(text));
  }
});

test("'string' is only a type now (no namespace entry, no sortText games)", async () => {
  const items = await complete('let a: st\n', 9);
  const entries = items.filter((i) => i.label === 'string');
  assert.strictEqual(entries.length, 1);
  assert.strictEqual(entries[0].kind, vscode.CompletionItemKind.TypeParameter);
  assert.strictEqual(entries[0].sortText, undefined);
});

(async () => {
  m.config = { path: SPG };
  const ext = require(EXT);
  await ext.activate({ subscriptions: [], extensionPath: '/nonexistent', extension: { packageJSON: { version: 'test' } } });
  await new Promise((r) => setTimeout(r, 300));
  let failed = 0;
  for (const [name, fn] of tests) {
    try { await fn(); console.log('  ok   ' + name); }
    catch (e) { failed++; console.log('  FAIL ' + name + '\n       ' + String(e.message).split('\n')[0]); }
  }
  await ext.deactivate();
  console.log(failed ? `${failed} FAILED` : `all ${tests.length} passed`);
  process.exit(failed ? 1 : 0);
})();
