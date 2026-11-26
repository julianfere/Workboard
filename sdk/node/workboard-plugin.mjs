// Workboard plugin SDK for Node.js (18+, no dependencies, ES module, single file).
//
// Copy this file next to your plugin and import it. It speaks Workboard's plugin protocol (JSON-RPC 2.0,
// one object per line on stdin/stdout, see docs/plugins.md): it answers `initialize`, routes
// `action/open`, `action/submit` and `tracker/*` calls to your handlers (sync or async), lets them call
// Workboard (`ctx.host.*`, promises) while they run and turns thrown errors into JSON-RPC errors.
//
//   import { Plugin, form, text, button, result } from "./workboard-plugin.mjs";
//
//   const plugin = new Plugin();
//   plugin.action("greet", {
//     open: (ctx) => form("Hello", [text("name", "Your name", { required: true })],
//                         { buttons: [button("greet", "Greet", { primary: true })] }),
//     submit: async (ctx, button, values, state) => result("Hello", `Hello, ${values.name}!`),
//   });
//   plugin.run();
//
// While the plugin runs, console.log and process.stdout.write go to stderr (Workboard's log) so they
// cannot break the protocol stream.

import { createInterface } from "node:readline";

/** Newest protocol version this SDK speaks. */
export const PROTOCOL = 1;
export const VERSION = "1.0.0";

const ERROR_FAILED = -32000;
const ERROR_METHOD = -32601;

const TRACKER_METHODS = {
  "tracker/connect": "connect",
  "tracker/states": "states",
  "tracker/fields": "fields",
  "tracker/workItems": "workItems",
  "tracker/assignedToMe": "assignedToMe",
  "tracker/detail": "detail",
  "tracker/setState": "setState",
};

// ---------------------------------------------------------------------------------------------- errors

/** An error whose message is shown to the user as is (no stack is logged). */
export class PluginError extends Error {
  constructor(message) {
    super(message);
    this.name = "PluginError";
  }
}

/**
 * A tracker error. Its JSON-RPC `message` is `code` when given, so Workboard can tell `tracker.auth`
 * (rejected credential) and `tracker.notFound` (missing project) apart; `message` then travels as the
 * error's `data.detail`.
 */
export class TrackerError extends PluginError {
  constructor(code, message) {
    super(code || message || "tracker error");
    this.name = "TrackerError";
    this.code = code || null;
    this.detail = message || code || "tracker error";
  }
}

/** Workboard answered a `host/...` request with an error; `code` is its message (`plugin.denied`…). */
export class HostError extends Error {
  constructor(method, code) {
    super(`${method} failed: ${code}`);
    this.name = "HostError";
    this.method = method;
    this.code = code;
  }
}

// -------------------------------------------------------------------------------------------- builders

function clean(o) {
  const out = {};
  for (const [k, v] of Object.entries(o)) if (v !== undefined && v !== null && v !== false) out[k] = v;
  return out;
}

function field(type, id, label, opts = {}, extra = {}) {
  const { value, required, wide, hint, placeholder, refresh } = opts;
  return clean({ id, type, label, value, required, wide, hint, placeholder, refresh, ...extra });
}

/** A single-line text field. Options: value, required, wide, hint, placeholder, refresh. */
export const text = (id, label, opts) => field("text", id, label, opts);

/** A multi-line text field; `rows` is its height. */
export const textarea = (id, label, opts = {}) => field("textarea", id, label, opts, { rows: opts.rows });

/** A password field; its value only comes back when the user typed something. */
export const password = (id, label, opts = {}) => field("password", id, label, { ...opts, value: undefined });

function option(o) {
  if (Array.isArray(o)) return { value: o[0], label: o.length > 1 ? o[1] : String(o[0]) };
  if (o && typeof o === "object") return o;
  return { value: o, label: String(o) };
}

/** A drop-down. `options` are `{value, label}` objects, `[value, label]` pairs or plain values. */
export const select = (id, label, options, opts = {}) =>
  field("select", id, label, opts, { options: options.map(option) });

/** A checkbox; its value comes back as a boolean. */
export function checkbox(id, label, opts = {}) {
  const f = field("checkbox", id, label, { ...opts, required: false, placeholder: undefined });
  f.value = Boolean(opts.value);
  return f;
}

/** A number field; its value comes back as a number. */
export const number = (id, label, opts) => field("number", id, label, opts);

/** A paragraph of plain text inside the form. Options: id, wide. */
export const info = (textValue, opts = {}) => clean({ type: "info", text: textValue, id: opts.id, wide: opts.wide });

/** A form or result button. Options: primary, validate (false skips required checks), confirm. */
export function button(id, label, opts = {}) {
  const b = clean({ id, label, primary: opts.primary, confirm: opts.confirm });
  if (opts.validate !== undefined) b.validate = Boolean(opts.validate);
  return b;
}

const toButton = (b) => (Array.isArray(b) ? button(...b) : b);

/**
 * A form step. Options: buttons (`button()` objects or `[id, label]` pairs; Cancel is always added),
 * intro, error, state (comes back untouched in `submit`).
 */
export function form(title, fields, opts = {}) {
  return clean({
    type: "form",
    title,
    intro: opts.intro,
    error: opts.error,
    fields: [...fields],
    buttons: (opts.buttons || []).map(toButton),
    state: opts.state,
  });
}

const toStep = (s) =>
  Array.isArray(s) ? clean({ label: s[0], ok: s.length > 1 ? Boolean(s[1]) : true, error: s[2] }) : s;
const fixOk = (s) => ({ ...s, ok: Boolean(s.ok) });
const toLink = (l) => (Array.isArray(l) ? { label: l[0], url: l[1] } : l);

/**
 * A result step. Options: ok (default true), steps (objects or `[label, ok, error?]`), links (objects or
 * `[label, url]`, only https:// is shown), buttons, state.
 */
export function result(title, message, opts = {}) {
  const r = { type: "result", title, ok: opts.ok === undefined ? true : Boolean(opts.ok) };
  if (message !== undefined && message !== null) r.message = message;
  if (opts.steps?.length) r.steps = opts.steps.map(toStep).map(fixOk);
  if (opts.links?.length) r.links = opts.links.map(toLink);
  if (opts.buttons?.length) r.buttons = opts.buttons.map(toButton);
  if (opts.state !== undefined && opts.state !== null) r.state = opts.state;
  return r;
}

/** Opens an agent session on `items` (item names). Options: prompt, name, mode (default/plan/acceptEdits), model. */
export function session(items, opts = {}) {
  return clean({
    type: "session",
    items: [...items],
    name: opts.name,
    mode: opts.mode || "default",
    prompt: opts.prompt,
    model: opts.model,
  });
}

/** Closes the dialog and shows `message`. */
export const close = (message) => clean({ type: "close", message });

// ------------------------------------------------------------------------------------------ host / ctx

/** Requests to Workboard while a call is handled. Every method returns a promise that rejects with a HostError. */
export class Host {
  constructor(rpc) {
    this._rpc = rpc;
  }
  /** Sends any `host/...` request. */
  request(method, params = {}) {
    return this._rpc.request(method, params);
  }
  workflow() {
    return this.request("host/workflow");
  }
  items() {
    return this.request("host/items");
  }
  item(name) {
    return this.request("host/item", { name });
  }
  /** Needs `items.write`; `""` deletes a field. */
  updateItem(name, fields) {
    return this.request("host/updateItem", { name, fields });
  }
  /** Needs `files.read`; resolves to the file's text. */
  async readFile(path) {
    return (await this.request("host/readFile", { path }))?.text;
  }
  async secretGet(key) {
    return (await this.request("host/secret/get", { key }))?.value ?? null;
  }
  async secretSet(key, value) {
    await this.request("host/secret/set", { key, value });
  }
  async secretDelete(key) {
    await this.request("host/secret/delete", { key });
  }
  dataGet() {
    return this.request("host/data/get");
  }
  async dataSet(data) {
    await this.request("host/data/set", { data });
  }
  /** Needs `tracker`: `{kind, source, org, project}` or null. */
  tracker() {
    return this.request("host/tracker");
  }
  async trackerWriteTokenGet() {
    return (await this.request("host/tracker/writeToken/get"))?.value ?? null;
  }
  async trackerWriteTokenSet(value) {
    await this.request("host/tracker/writeToken/set", { value });
  }
  async trackerWriteTokenDelete() {
    await this.request("host/tracker/writeToken/delete");
  }
}

/** Where an action runs: lang, root, placement, items, item (the first), pluginDir, pluginId, action, host. */
export class Context {
  constructor(plugin, action, context = {}) {
    context = context || {};
    this.action = action;
    this.lang = context.lang || plugin.lang;
    this.root = context.root ?? null;
    this.placement = context.placement ?? null;
    this.items = [...(context.items || [])];
    this.item = this.items[0] ?? null;
    this.raw = context;
    this.pluginDir = plugin.pluginDir;
    this.pluginId = plugin.pluginId;
    this.host = plugin.host;
  }
}

// ------------------------------------------------------------------------------------------------- rpc

class Rpc {
  constructor(input, write) {
    this.write = write;
    this.next = 1000;
    this.waiting = new Map();
    this.closed = false;
    this.onRequest = () => {};
    this.onClose = () => {};
    const rl = createInterface({ input, crlfDelay: Infinity });
    rl.on("line", (line) => this.line(line));
    rl.on("close", () => {
      this.closed = true;
      for (const [, w] of this.waiting) w.reject(new HostError(w.method, "Workboard closed the connection"));
      this.waiting.clear();
      this.onClose();
    });
  }

  line(raw) {
    const line = raw.trim();
    if (!line) return;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      process.stderr.write("workboard-plugin: ignoring a line that is not JSON\n");
      return;
    }
    if (!msg || typeof msg !== "object") return;
    if (typeof msg.method === "string") return this.onRequest(msg);
    const w = this.waiting.get(msg.id);
    if (!w) return;
    this.waiting.delete(msg.id);
    if (msg.error != null) w.reject(new HostError(w.method, msg.error.message ?? String(msg.error)));
    else w.resolve(msg.result ?? null);
  }

  send(msg) {
    return this.write(JSON.stringify(msg) + "\n");
  }

  request(method, params) {
    if (this.closed) return Promise.reject(new HostError(method, "Workboard closed the connection"));
    const id = ++this.next;
    return new Promise((resolve, reject) => {
      this.waiting.set(id, { method, resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }
}

function redact(textValue, secret) {
  return typeof secret === "string" && secret.length >= 4 ? textValue.split(secret).join("***") : textValue;
}

// ---------------------------------------------------------------------------------------------- plugin

class MethodNotFound extends Error {}

/**
 * A Workboard plugin: register handlers with `action()` and `tracker()`, then call `run()`. After
 * `initialize`, `lang`, `pluginId`, `pluginDir` and `host` are set.
 */
export class Plugin {
  constructor() {
    this.actions = new Map();
    this.trackers = new Map();
    this.lang = "en";
    this.pluginId = null;
    this.pluginDir = null;
    this.init = {};
    this.host = null;
  }

  /** Registers action `id`: `{ open(ctx), submit(ctx, button, values, state) }`, each returning a step. */
  action(id, handlers) {
    if (typeof handlers === "function") handlers = { open: handlers };
    this.actions.set(id, handlers);
    return this;
  }

  /**
   * Registers tracker `kind`: an object (or class, instantiated once) with `connect`, `states`, `fields`,
   * `workItems`, `assignedToMe`, `detail` and `setState`, each taking `(conn, secret, …)`.
   */
  tracker(kind, impl) {
    this.trackers.set(kind, typeof impl === "function" ? new impl() : impl);
    return this;
  }

  initialize(params = {}) {
    const version = params.protocol ?? PROTOCOL;
    if (Number.isInteger(version) && version > PROTOCOL) {
      throw new PluginError(`protocol ${version} is newer than this plugin supports (${PROTOCOL}): update the plugin`);
    }
    this.init = params;
    this.lang = params.lang || "en";
    this.pluginId = params.plugin?.id ?? null;
    this.pluginDir = params.plugin?.dir ?? null;
    return {};
  }

  async callAction(method, params) {
    const a = this.actions.get(params.action);
    if (!a) throw new PluginError(`unknown action ${JSON.stringify(params.action)}`);
    const ctx = new Context(this, params.action, params.context);
    let step;
    if (method === "action/open") {
      step = await a.open(ctx);
    } else {
      if (!a.submit) throw new PluginError(`action ${JSON.stringify(params.action)} has no submit handler`);
      step = await a.submit(ctx, params.button ?? null, params.values || {}, params.state || {});
    }
    if (!step || typeof step !== "object") throw new PluginError(`action ${JSON.stringify(params.action)} returned no step`);
    return step;
  }

  async callTracker(method, params) {
    let impl = this.trackers.get(params.kind);
    if (!impl && params.kind == null && this.trackers.size === 1) impl = [...this.trackers.values()][0];
    if (!impl) throw new TrackerError(null, `unknown tracker kind ${JSON.stringify(params.kind)}`);
    const name = TRACKER_METHODS[method];
    if (typeof impl[name] !== "function") throw new PluginError(`${method} is not implemented`);
    const conn = { org: "", project: "", ...(params.connection || {}) };
    const secret = params.secret ?? null;
    const args = {
      "tracker/workItems": [params.ids || [], params.request || {}],
      "tracker/assignedToMe": [params.dueField ?? null, params.start ?? null, params.end ?? null, params.exclude || []],
      "tracker/detail": [params.id],
      "tracker/setState": [params.id, params.state],
    }[method] || [];
    let out = await impl[name](conn, secret, ...args);
    if (out == null && method === "tracker/connect") out = { org: conn.org, project: conn.project };
    return out ?? null;
  }

  /** Answers one call; rejects on errors. `run()` uses it. */
  async handle(method, params) {
    params = params && typeof params === "object" ? params : {};
    if (method === "initialize") return this.initialize(params);
    if (method === "action/open" || method === "action/submit") return this.callAction(method, params);
    if (method in TRACKER_METHODS) return this.callTracker(method, params);
    throw new MethodNotFound(method);
  }

  async answer(msg) {
    const { id, method } = msg;
    const secret = msg.params?.secret;
    let reply;
    try {
      reply = { jsonrpc: "2.0", id, result: (await this.handle(method, msg.params)) ?? null };
    } catch (e) {
      let error;
      if (e instanceof MethodNotFound) {
        error = { code: ERROR_METHOD, message: `unknown method ${method}` };
      } else if (e instanceof TrackerError) {
        error = { code: ERROR_FAILED, message: redact(e.code || e.detail, secret) };
        if (e.code && e.detail !== e.code) error.data = { detail: redact(e.detail, secret) };
      } else if (e instanceof PluginError || e instanceof HostError) {
        error = { code: ERROR_FAILED, message: redact(e.message, secret) };
      } else {
        this.stderr(redact(String(e?.stack || e), secret) + "\n");
        const name = e?.name || "Error";
        const message = e?.message ?? String(e);
        error = { code: ERROR_FAILED, message: redact(message ? `${name}: ${message}` : name, secret) };
      }
      reply = { jsonrpc: "2.0", id, error };
    }
    if (id !== undefined && id !== null) await this.rpc.send(reply);
  }

  /**
   * Serves Workboard until it closes stdin, then exits. Options `{input, output}` (streams) replace the
   * process's stdio, for tests; then the process is not exited and the returned promise resolves instead.
   */
  run({ input, output } = {}) {
    const ownStdio = !input && !output;
    input = input || process.stdin;
    output = output || process.stdout;
    let rawWrite = output.write.bind(output);
    if (ownStdio) {
      // Stray output goes to the log, not to the protocol stream.
      process.stdout.write = (...args) => process.stderr.write(...args);
      console.log = console.info = console.debug = (...args) => console.error(...args);
    }
    this.stderr = (s) => process.stderr.write(s);
    const write = (s) => new Promise((resolve) => rawWrite(s, "utf8", () => resolve()));
    this.rpc = new Rpc(input, write);
    this.host = new Host(this.rpc);
    let chain = Promise.resolve();
    this.rpc.onRequest = (msg) => {
      chain = chain.then(() => this.answer(msg));
    };
    return new Promise((resolve) => {
      this.rpc.onClose = () => {
        chain.then(() => {
          resolve();
          if (ownStdio) process.exit(0);
        });
      };
    });
  }
}
