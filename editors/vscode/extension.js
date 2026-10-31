'use strict';
// Spaghetti extension for VSCode — no dependencies (no node_modules).
// The "LSP client" below is hand-written: it spawns `spg lsp`, speaks JSON-RPC
// over stdin/stdout and translates to the VSCode API.

const vscode = require('vscode');
const cp = require('child_process');
const fs = require('fs');
const path = require('path');

const LANG = 'spg';
const DEBOUNCE_MS = 150;
const REQUEST_TIMEOUT_MS = 10000;

// ───────────────────────── LSP client ─────────────────────────

class LspClient {
  constructor(command, cwd, log, trace) {
    this.command = command;
    this.cwd = cwd;
    this.log = log;
    this.trace = trace;
    this.nextId = 1;
    this.pending = new Map();
    this.buf = Buffer.alloc(0);
    this.handlers = new Map(); // method -> fn(params)
    this.proc = null;
    this.alive = false;
    this.onExit = () => {};
  }

  on(method, fn) { this.handlers.set(method, fn); }

  start() {
    return new Promise((resolve, reject) => {
      let settled = false;
      const proc = cp.spawn(this.command, ['lsp'], {
        cwd: this.cwd, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
      });
      this.proc = proc;
      proc.on('error', (err) => {
        this.alive = false;
        if (!settled) { settled = true; reject(err); }
      });
      proc.on('spawn', () => {
        this.alive = true;
        if (!settled) { settled = true; resolve(); }
      });
      proc.stdout.on('data', (chunk) => this._onData(chunk));
      proc.stderr.on('data', (d) => this.log(`[stderr] ${d.toString('utf8').trimEnd()}`));
      proc.stdin.on('error', () => { /* the process died: 'exit' handles it */ });
      proc.on('exit', (code, signal) => {
        this.alive = false;
        for (const { reject: rj, timer } of this.pending.values()) { clearTimeout(timer); rj(new Error('server exited')); }
        this.pending.clear();
        this.onExit(code, signal);
      });
    });
  }

  _onData(chunk) {
    this.buf = Buffer.concat([this.buf, chunk]);
    for (;;) {
      const sep = this.buf.indexOf('\r\n\r\n');
      if (sep < 0) return;
      const header = this.buf.slice(0, sep).toString('ascii');
      const m = /content-length:\s*(\d+)/i.exec(header);
      if (!m) { this.buf = this.buf.slice(sep + 4); continue; } // invalid header: drop it
      const len = parseInt(m[1], 10);
      const end = sep + 4 + len;
      if (this.buf.length < end) return; // incomplete body: wait for more bytes
      const body = this.buf.slice(sep + 4, end).toString('utf8');
      this.buf = this.buf.slice(end);
      let msg;
      try { msg = JSON.parse(body); } catch (e) { this.log(`invalid JSON from server: ${e.message}`); continue; }
      if (this.trace) this.log(`<-- ${body}`);
      this._dispatch(msg);
    }
  }

  _dispatch(msg) {
    if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined) && !msg.method) {
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      clearTimeout(p.timer);
      if (msg.error) p.reject(new Error(msg.error.message || 'server error'));
      else p.resolve(msg.result);
      return;
    }
    const h = this.handlers.get(msg.method);
    if (h) { try { h(msg.params); } catch (e) { this.log(`error in ${msg.method}: ${e.stack || e}`); } }
  }

  _send(obj) {
    if (!this.alive || !this.proc || !this.proc.stdin.writable) return false;
    const body = Buffer.from(JSON.stringify(obj), 'utf8');
    if (this.trace) this.log(`--> ${body.toString('utf8')}`);
    this.proc.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, 'ascii'), body]));
    return true;
  }

  notify(method, params) { return this._send({ jsonrpc: '2.0', method, params }); }

  request(method, params) {
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`timeout in ${method}`)); }, REQUEST_TIMEOUT_MS);
      this.pending.set(id, { resolve, reject, timer });
      if (!this._send({ jsonrpc: '2.0', id, method, params })) {
        clearTimeout(timer); this.pending.delete(id); reject(new Error('server is not running'));
      }
    });
  }

  async stop() {
    if (!this.proc) return;
    const proc = this.proc;
    try {
      if (this.alive) {
        await Promise.race([this.request('shutdown', null), new Promise((r) => setTimeout(r, 1500))]);
        this.notify('exit', null);
      }
    } catch (_) { /* we kill it anyway */ }
    await new Promise((resolve) => {
      if (proc.exitCode !== null) return resolve();
      const t = setTimeout(() => { try { proc.kill(); } catch (_) {} resolve(); }, 1500);
      proc.once('exit', () => { clearTimeout(t); resolve(); });
    });
  }
}

// ───────────────────────── extension state ─────────────────────────

let context = null;
let output = null;
let diagnostics = null;
let statusItem = null;
let client = null;
let starting = false;
let stopping = false;
const opened = new Set();             // uris (strings) already announced to the server
const debounce = new Map();           // uri -> timer
const restarts = [];                  // timestamps of automatic restarts

function log(msg) { if (output) output.appendLine(`[${new Date().toISOString().slice(11, 19)}] ${msg}`); }
function cfg() { return vscode.workspace.getConfiguration('spg'); }
function isSpg(doc) { return doc && doc.languageId === LANG && doc.uri.scheme === 'file'; }

/** setting > binary bundled with the extension > `spg` from PATH */
function resolveBinary() {
  const configured = (cfg().get('path') || '').trim();
  if (configured) return configured;
  const exe = process.platform === 'win32' ? 'spg.exe' : 'spg';
  const bundled = path.join(context.extensionPath, 'bin', exe);
  if (fs.existsSync(bundled)) return bundled;
  return 'spg';
}

function setStatus(state, detail) {
  if (!statusItem) return;
  const icons = { on: '$(check)', off: '$(circle-slash)', error: '$(error)', starting: '$(sync~spin)' };
  statusItem.text = `${icons[state] || ''} spg`;
  statusItem.tooltip = detail || '';
  statusItem.backgroundColor = state === 'error' ? new vscode.ThemeColor('statusBarItem.errorBackground') : undefined;
}

// ───────────────────────── document sync ─────────────────────────

function sendOpen(doc) {
  if (!client || !client.alive) return;
  const uri = doc.uri.toString();
  client.notify('textDocument/didOpen', {
    textDocument: { uri, languageId: LANG, version: doc.version, text: doc.getText() },
  });
  opened.add(uri);
}

function flush(doc) {
  const uri = doc.uri.toString();
  const t = debounce.get(uri);
  if (t) { clearTimeout(t); debounce.delete(uri); }
  if (!client || !client.alive || !opened.has(uri)) return;
  client.notify('textDocument/didChange', {
    textDocument: { uri, version: doc.version },
    contentChanges: [{ text: doc.getText() }],
  });
}

function scheduleChange(doc) {
  const uri = doc.uri.toString();
  const old = debounce.get(uri);
  if (old) clearTimeout(old);
  debounce.set(uri, setTimeout(() => { debounce.delete(uri); flush(doc); }, DEBOUNCE_MS));
}

function toDiagnostic(d) {
  const r = d.range;
  const diag = new vscode.Diagnostic(
    new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character),
    d.message,
    d.severity === 2 ? vscode.DiagnosticSeverity.Warning
      : d.severity === 3 ? vscode.DiagnosticSeverity.Information
      : d.severity === 4 ? vscode.DiagnosticSeverity.Hint
      : vscode.DiagnosticSeverity.Error,
  );
  diag.source = d.source || 'spg';
  return diag;
}

// ui snippets: read from snippets/ui.json at activation. They are not contributed in package.json,
// because VS Code would offer them in every .spg file; here they only appear once `install #ui` is in the document.
const UI_INSTALL = /^\s*install\s*#ui\b/m;
let uiSnippets = [];

function loadUiSnippets(ctx) {
  try {
    const file = path.join(ctx.extensionPath, 'snippets', 'ui.json');
    const all = JSON.parse(fs.readFileSync(file, 'utf8'));
    uiSnippets = Object.entries(all).map(([name, s]) => ({ name, ...s }));
  } catch (e) {
    log(`ui snippets not loaded: ${e.message}`);
  }
}

/** the ui snippets for this document; none after `.`, `@`, `#` or `>` (those open other lists) */
function uiSnippetItems(doc, trigger) {
  if (!uiSnippets.length || ['.', '@', '#', '>'].includes(trigger)) return [];
  if (!UI_INSTALL.test(doc.getText())) return [];
  return uiSnippets.map((s) => {
    const prefixes = Array.isArray(s.prefix) ? s.prefix : [s.prefix];
    const item = new vscode.CompletionItem(prefixes[0], vscode.CompletionItemKind.Snippet);
    item.insertText = new vscode.SnippetString(Array.isArray(s.body) ? s.body.join('\n') : s.body);
    item.filterText = prefixes.join(' ');
    item.detail = s.name;
    item.documentation = s.description;
    return item;
  });
}

/** LSP CompletionItem -> vscode.CompletionItem */
function toCompletionItem(i) {
  const item = new vscode.CompletionItem(i.label, i.kind ? i.kind - 1 : undefined); // LSP kinds start at 1
  if (i.detail) item.detail = i.detail;
  if (i.insertText) item.insertText = i.insertText;   // a namespace inserts "@math." (with the dot)
  if (i.textEdit && i.textEdit.range) {               // ...replacing the '@' and the partial name already typed
    const r = i.textEdit.range;
    item.range = new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);
    item.insertText = i.textEdit.newText;
  }
  if (i.sortText) item.sortText = i.sortText;
  if (i.filterText) item.filterText = i.filterText;
  if (i.command && i.command.command) {               // ...and reopens the list to show its members
    item.command = { command: i.command.command, title: i.command.title || '', arguments: i.command.arguments };
  }
  return item;
}

// ───────────────────────── server lifecycle ─────────────────────────

async function startServer() {
  if (starting || (client && client.alive)) return;
  if (!cfg().get('server.enable')) { setStatus('off', 'Language server is disabled in the settings'); return; }
  starting = true;
  setStatus('starting', 'Starting the language server...');
  const command = resolveBinary();
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  const c = new LspClient(command, folder ? folder.uri.fsPath : undefined, log, cfg().get('trace.server') === 'messages');
  c.on('textDocument/publishDiagnostics', (p) => {
    diagnostics.set(vscode.Uri.parse(p.uri), (p.diagnostics || []).map(toDiagnostic));
  });
  c.onExit = (code, signal) => {
    if (client !== c) return;
    log(`server exited (code ${code}, signal ${signal})`);
    opened.clear();
    if (stopping) return;
    const now = Date.now();
    while (restarts.length && now - restarts[0] > 60000) restarts.shift();
    if (restarts.length >= 3) {
      setStatus('error', 'The server crashed 3 times in 1 minute. Check the log.');
      vscode.window.showErrorMessage('The Spaghetti language server keeps crashing.', 'Show log')
        .then((x) => { if (x) output.show(); });
      return;
    }
    restarts.push(now);
    client = null;
    setTimeout(() => { startServer(); }, 500);
  };
  client = c;
  try {
    await c.start();
    await c.request('initialize', {
      processId: process.pid,
      rootUri: folder ? folder.uri.toString() : null,
      capabilities: {},
      clientInfo: { name: 'vscode-spaghetti', version: context.extension.packageJSON.version },
    });
    c.notify('initialized', {});
    log(`server started: ${command} lsp`);
    setStatus('on', `Server running\n${command}`);
    for (const doc of vscode.workspace.textDocuments) if (isSpg(doc)) sendOpen(doc);
  } catch (err) {
    client = null;
    const missing = err && err.code === 'ENOENT';
    log(`failed to start '${command}': ${err && err.message}`);
    setStatus('error', missing ? `Executable not found: ${command}` : String(err && err.message));
    if (missing) {
      const pick = await vscode.window.showErrorMessage(
        `Could not find the Spaghetti executable ('${command}'). Install spg or set its path.`,
        'Set path', 'Show log');
      if (pick === 'Set path') vscode.commands.executeCommand('workbench.action.openSettings', 'spg.path');
      else if (pick === 'Show log') output.show();
    }
  } finally {
    starting = false;
  }
}

async function stopServer() {
  const c = client;
  if (!c) return;
  stopping = true;
  try { await c.stop(); } finally { stopping = false; }
  if (client === c) client = null;
  opened.clear();
  diagnostics.clear();
}

// ───────────────────────── run / check ─────────────────────────

function targetDocument(arg) {
  if (arg instanceof vscode.Uri) return arg;
  const ed = vscode.window.activeTextEditor;
  return ed && ed.document.languageId === LANG ? ed.document.uri : null;
}

async function runSpg(sub, arg) {
  const uri = targetDocument(arg);
  if (!uri || uri.scheme !== 'file') {
    vscode.window.showWarningMessage('Open a .spg file saved on disk to use this command.');
    return;
  }
  const open = vscode.workspace.textDocuments.find((d) => d.uri.toString() === uri.toString());
  if (open && open.isDirty) await open.save();

  const exe = resolveBinary();
  const strong = vscode.ShellQuoting.Strong;
  const exec = new vscode.ShellExecution({ value: exe, quoting: strong }, [sub, { value: uri.fsPath, quoting: strong }], {
    cwd: path.dirname(uri.fsPath),
  });
  const task = new vscode.Task({ type: 'spg' }, vscode.TaskScope.Workspace,
    `${sub === 'run' ? 'Run' : 'Check'} ${path.basename(uri.fsPath)}`, 'spg', exec, []);
  task.presentationOptions = { reveal: vscode.TaskRevealKind.Always, clear: false, focus: false, panel: vscode.TaskPanelKind.Shared };
  await vscode.tasks.executeTask(task);
}

// ───────────────────────── activation ─────────────────────────

async function activate(ctx) {
  loadUiSnippets(ctx);
  context = ctx;
  output = vscode.window.createOutputChannel('Spaghetti');
  diagnostics = vscode.languages.createDiagnosticCollection('spg');
  statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 50);
  statusItem.command = 'spg.showOutput';
  statusItem.show();
  ctx.subscriptions.push(output, diagnostics, statusItem);

  ctx.subscriptions.push(
    vscode.commands.registerCommand('spg.run', (arg) => runSpg('run', arg)),
    vscode.commands.registerCommand('spg.check', (arg) => runSpg('check', arg)),
    vscode.commands.registerCommand('spg.showOutput', () => output.show()),
    vscode.commands.registerCommand('spg.restartServer', async () => {
      restarts.length = 0;
      await stopServer();
      await startServer();
    }),

    vscode.workspace.onDidOpenTextDocument((doc) => { if (isSpg(doc)) sendOpen(doc); }),
    vscode.workspace.onDidChangeTextDocument((e) => { if (isSpg(e.document)) scheduleChange(e.document); }),
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (!isSpg(doc) || !client || !client.alive) return;
      flush(doc);
      client.notify('textDocument/didSave', { textDocument: { uri: doc.uri.toString() } });
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      if (!isSpg(doc)) return;
      const uri = doc.uri.toString();
      const t = debounce.get(uri);
      if (t) { clearTimeout(t); debounce.delete(uri); }
      if (client && client.alive && opened.has(uri)) client.notify('textDocument/didClose', { textDocument: { uri } });
      opened.delete(uri);
      diagnostics.delete(doc.uri);
    }),
    vscode.workspace.onDidChangeConfiguration(async (e) => {
      if (e.affectsConfiguration('spg.path') || e.affectsConfiguration('spg.server.enable') || e.affectsConfiguration('spg.trace.server')) {
        restarts.length = 0;
        await stopServer();
        await startServer();
      }
    }),

    vscode.languages.registerCompletionItemProvider(LANG, {
      async provideCompletionItems(doc, position, _token, context) {
        // the ui snippets are only for documents that install ui (they are not in the global snippet list)
        const snippetItems = uiSnippetItems(doc, context && context.triggerCharacter);
        if (!client || !client.alive) return snippetItems.length ? snippetItems : undefined;
        // '#' opens the list only right after `install #` (the libraries); anywhere else
        // (a comment, a string, a `#>` directive) it stays quiet
        // '#' opens the libraries only right after `install #`; '>' opens the directives only
        // when the line is just `#>`. anywhere else (a comment, a string, `a > b`) they stay quiet
        if (context && (context.triggerCharacter === '#' || context.triggerCharacter === '>')) {
          const before = doc.lineAt(position.line).text.slice(0, position.character);
          const ok = context.triggerCharacter === '#' ? /^\s*install\s*#$/.test(before) : /^\s*#>$/.test(before);
          if (!ok) return undefined;
        }
        flush(doc);
        try {
          const res = await client.request('textDocument/completion', {
            textDocument: { uri: doc.uri.toString() },
            position: { line: position.line, character: position.character },
          });
          return [...(res || []).map(toCompletionItem), ...snippetItems];
        } catch (e) {
          log(`completion failed: ${e.message}`);
          return snippetItems.length ? snippetItems : undefined;
        }
      },
    }, '.', '@', '#', '>'), // '.' opens the member list as soon as it is typed; '@' opens the namespace list; '#' and '>' open the libraries (after `install #`) and the directives (after `#>`)

    // `Format Document` and format on save: the server answers with one edit that replaces
    // the whole file, or with none when the file is already formatted or cannot be formatted
    // safely (for example it has a syntax error)
    vscode.languages.registerDocumentFormattingEditProvider(LANG, {
      async provideDocumentFormattingEdits(doc) {
        if (!client || !client.alive) return undefined;
        flush(doc);
        try {
          const res = await client.request('textDocument/formatting', {
            textDocument: { uri: doc.uri.toString() },
            options: { tabSize: 4, insertSpaces: true },
          });
          return (res || []).map((e) => new vscode.TextEdit(
            new vscode.Range(e.range.start.line, e.range.start.character, e.range.end.line, e.range.end.character),
            e.newText));
        } catch (e) {
          log(`formatting failed: ${e.message}`);
          return undefined;
        }
      },
    }),
  );

  await startServer();
}

async function deactivate() {
  for (const t of debounce.values()) clearTimeout(t);
  debounce.clear();
  await stopServer();
}

module.exports = { activate, deactivate, LspClient };
