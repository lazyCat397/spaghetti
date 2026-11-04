'use strict';
const fs = require('fs');
const path = require('path');
const vsctm = require('vscode-textmate');
const oniguruma = require('vscode-oniguruma');

const GRAMMAR = path.join(__dirname, '..', '..', 'editors', 'vscode', 'syntaxes', 'spg.tmLanguage.json');
const wasm = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm')).buffer;
const onigLib = oniguruma.loadWASM(wasm).then(() => ({
  createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
  createOnigString: (s) => new oniguruma.OnigString(s),
}));
const registry = new vsctm.Registry({
  onigLib,
  loadGrammar: async (scope) => (scope === 'source.spg' ? vsctm.parseRawGrammar(fs.readFileSync(GRAMMAR, 'utf8'), GRAMMAR) : null),
});

// the scopes of the token that covers the `nth` occurrence of `needle` in `line`
function scopesAt(grammar, line, needle, nth) {
  let from = 0;
  let idx = -1;
  for (let i = 0; i <= nth; i++) {
    idx = line.indexOf(needle, from);
    if (idx < 0) return null;
    from = idx + 1;
  }
  const { tokens } = grammar.tokenizeLine(line, vsctm.INITIAL);
  const t = tokens.find((tok) => tok.startIndex <= idx && idx < tok.endIndex);
  return t ? t.scopes : null;
}

// [name, line, needle, nth occurrence, scope it must have, scope it must NOT have]
const cases = [
  // struct guards
  ['guard is a keyword', 'struct S { guard { } }', 'guard', 0, 'keyword.control.guard.spg'],
  ['guard with a field is a keyword', '    guard (me.grade as g) {', 'guard', 0, 'keyword.control.guard.spg'],
  ['me in a guard is still the variable', '    guard (me.grade as g) {', 'me', 0, 'variable.language.me.spg'],
  ['guard does not match inside a longer name', 'let guardian = 1', 'guardian', 0, null, 'keyword.control.guard.spg'],
  // none and optional types
  ['none is a language constant', 'let x: int? = none', 'none', 0, 'constant.language.none.spg'],
  ['none is matched as a whole word only', 'let nonexistent = 1', 'nonexistent', 0, null, 'constant.language.none.spg'],
  ['the ? of an optional type', 'let x: int? = none', '?', 0, 'keyword.operator.optional.spg'],
  ['optional struct field', '    prox: No?', '?', 0, 'keyword.operator.optional.spg'],
  ['optional generic argument', 'let m: map<string, int?> = [:]', '?', 0, 'keyword.operator.optional.spg'],
  ['?? is ONE coalesce token', 'let y = a ?? b', '??', 0, 'keyword.operator.coalesce.spg', 'keyword.operator.optional.spg'],
  // interpolation
  ['interpolation opens', 'print("hi ${name}!")', '${', 0, 'punctuation.section.interpolation.begin.spg'],
  ['interpolation body is embedded code', 'print("hi ${name}!")', 'name', 0, 'meta.embedded.line.spg'],
  ['interpolation closes', 'print("hi ${name}!")', '}', 0, 'punctuation.section.interpolation.end.spg'],
  ['text after the interpolation is a string again', 'print("hi ${name}!")', '!', 0, 'string.quoted.double.spg', 'meta.embedded.line.spg'],
  ['text before the interpolation is a string', 'print("hi ${name}!")', 'hi', 0, 'string.quoted.double.spg', 'meta.embedded.line.spg'],
  ['operators highlight inside interpolation', 'print("${a ?? 0}")', '??', 0, 'keyword.operator.coalesce.spg'],
  ['numbers highlight inside interpolation', 'print("${a + 42}")', '42', 0, 'constant.numeric.integer.spg'],
  ['a string nested in an interpolation', 'print("a ${"b ${c}"} zzz")', 'b', 0, 'string.quoted.double.spg'],
  ['an interpolation nested in that string', 'print("a ${"b ${c}"} zzz")', 'c', 0, 'meta.embedded.line.spg'],
  ['the outer string resumes after nesting', 'print("a ${"b ${c}"} zzz")', 'zzz', 0, 'string.quoted.double.spg', 'meta.embedded.line.spg'],
  ['braces inside an interpolation', 'print("${ P { x: 1 }.x } end")', 'end', 0, 'string.quoted.double.spg', 'meta.embedded.line.spg'],
  ['a } inside a nested string does not close it', 'print("${@str.len("}")} end")', 'end', 0, 'string.quoted.double.spg', 'meta.embedded.line.spg'],
  ['an escaped \\${ is plain text', 'print("\\${x}")', '\\$', 0, 'constant.character.escape.spg'],
  ['after an escaped \\${ nothing is interpolated', 'print("\\${x}")', 'x', 0, 'string.quoted.double.spg', 'meta.interpolation.spg'],
  ['a lone $ is plain text', 'print("custa $5")', '$5', 0, 'string.quoted.double.spg', 'meta.interpolation.spg'],
  ['raw strings never interpolate', "print('cru ${x}')", 'x', 0, 'string.quoted.single.spg', 'meta.interpolation.spg'],
  ['a comment is not interpolated', '// hi ${x}', 'x', 0, 'comment.line.double-slash.spg', 'meta.interpolation.spg'],
  // functions as values
  ['a lambda keyword', 'let f = fn(x: int) -> int { return x }', 'fn', 0, 'storage.type.function.spg', 'entity.name.function.call.spg'],
  ['a lambda keyword with a space', 'let f = fn (x: int) -> int { return x }', 'fn', 0, 'storage.type.function.spg', 'entity.name.function.call.spg'],
  ['a fn type', 'fn ap(f: fn(int) -> int?) -> int? {', 'fn', 1, 'storage.type.function.spg', 'entity.name.function.call.spg'],
  ['a declared fn name', 'fn ap(f: fn(int) -> int?) -> int? {', 'ap', 0, 'entity.name.function.spg'],
  ['the ? of an optional return type', 'fn ap(f: fn(int) -> int?) -> int? {', '?', 1, 'keyword.operator.optional.spg'],
  ['an arrow', 'let f = fn(x: int) -> int { return x }', '->', 0, 'keyword.operator.arrow.spg'],
  // what must keep working
  ['calls', 'print(dobro(2))', 'dobro', 0, 'entity.name.function.call.spg'],
  ['calling a function value', 'let r = soma3(10)', 'soma3', 0, 'entity.name.function.call.spg'],
  ['builtins', 'print(1)', 'print', 0, 'support.function.builtin.spg'],
  ['namespace functions', 'let s = @str.len("a")', 'len', 0, 'support.function.builtin.spg'],
  ['floats', 'let n = 3.5', '3.5', 0, 'constant.numeric.float.spg'],
  ['a range is not a float', 'for i in 1..4 {', '1', 0, 'constant.numeric.integer.spg', 'constant.numeric.float.spg'],
  ['directives', '#> strict', 'strict', 0, 'keyword.other.directive.spg'],
  ['me', 'return me.n', 'me', 0, 'variable.language.me.spg'],
  ['step', 'for i in 5..0 step -2 {', 'step', 0, 'keyword.control.loop.step.spg'],
  ['true', 'let b = true', 'true', 0, 'constant.language.boolean.spg'],
  ['struct names', 'let p = Point { x: 1 }', 'Point', 0, 'entity.name.type.spg'],
  ['properties', 'print(p.x)', 'x', 0, 'variable.other.property.spg'],
  ['comments', 'let a = 1 // hi', '// hi', 0, 'comment.line.double-slash.spg'],
  // statement separator (optional, like JavaScript)
  ['; separates statements', 'let a = 1; let b = 2;', ';', 0, 'punctuation.terminator.statement.spg'],
  ['; closing a block line', '    print(a);', ';', 0, 'punctuation.terminator.statement.spg'],
  ['doubled ; is still a terminator', 'print(a);;', ';', 1, 'punctuation.terminator.statement.spg'],
  ['type is a builtin function', 'let k = type(a)', 'type', 0, 'support.function.builtin.spg'],
  ['Image is a native type', 'let pic: Image = win.image(0.0, 0.0, "a.png", 1)', 'Image', 0, 'entity.name.type.spg'],
  ['Window is a native type', 'let win: Window = ui.window("t", 8, 8)', 'Window', 0, 'entity.name.type.spg'],
  ['the image method is not a type', 'let pic = win.image(0.0, 0.0, "a.png", 1)', 'image', 0, null, 'entity.name.type.spg'],
  ['a name that contains Image is not a type', 'let myImage = 1', 'myImage', 0, null, 'entity.name.type.spg'],
];

(async () => {
  const grammar = await registry.loadGrammar('source.spg');
  let failed = 0;
  for (const [name, line, needle, nth, must, mustNot] of cases) {
    const scopes = scopesAt(grammar, line, needle, nth);
    let problem = null;
    if (!scopes) problem = `no token found for ${JSON.stringify(needle)} in ${JSON.stringify(line)}`;
    else if (must && !scopes.includes(must)) problem = `missing ${must}`;
    else if (mustNot && scopes.includes(mustNot)) problem = `must not have ${mustNot}`;
    if (problem) {
      failed++;
      console.log(`FAIL ${name}: ${problem}\n     line:   ${line}\n     scopes: ${scopes ? scopes.join(' ') : '-'}`);
    }
  }
  console.log(`grammar: ${cases.length - failed} ok, ${failed} failed`);

  // every example program must tokenize back to the top level: a rule that never closes
  // (an interpolation, a string) would color the rest of the file
  const examples = path.join(__dirname, '..', '..', 'examples');
  const files = [examples, path.join(examples, 'multi', 'ok')]
    .filter((d) => fs.existsSync(d))
    .flatMap((d) => fs.readdirSync(d).filter((f) => f.endsWith('.spg')).map((f) => path.join(d, f)));
  let leaked = 0;
  for (const file of files) {
    let stack = vsctm.INITIAL;
    for (const line of fs.readFileSync(file, 'utf8').split('\n')) stack = grammar.tokenizeLine(line, stack).ruleStack;
    if (stack.depth > 1) {
      leaked++;
      console.log(`FAIL a rule is still open at the end of ${path.relative(examples, file)}`);
    }
  }
  console.log(`examples: ${files.length - leaked} of ${files.length} tokenize back to the top level`);
  process.exit(failed || leaked ? 1 : 0);
})();
