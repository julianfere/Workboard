// Tests for workboard-plugin.mjs: the fixture plugin runs as a subprocess speaking the protocol, like
// Workboard runs it. Run with `node --test "sdk/node/tests/*.test.mjs"` (also part of `npm test`).
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import * as wb from "../workboard-plugin.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURE = join(HERE, "fixture-plugin.mjs");
const INIT = {
  protocol: 1,
  host: { name: "workboard", version: "0.1.0" },
  lang: "es",
  plugin: { id: "fixture", dir: HERE },
};
const CTX = { lang: "es", root: "/r", placement: "item", items: [{ name: "wi1-login", kind: "item" }] };
const CONN = { org: "acme/", project: "board" };

/** The fixture plugin as Workboard sees it. */
class Proc {
  constructor() {
    this.p = spawn(process.execPath, [FIXTURE], { stdio: ["pipe", "pipe", "pipe"] });
    this.lines = [];
    this.waiters = [];
    this.unread = 0;
    this.stderr = "";
    this.next = 0;
    createInterface({ input: this.p.stdout }).on("line", (l) => {
      const w = this.waiters.shift();
      if (w) w(l);
      else this.lines.push(l);
    });
    this.p.stderr.setEncoding("utf8").on("data", (d) => (this.stderr += d));
    this.exited = new Promise((resolve) => this.p.on("exit", (code) => resolve(code)));
    this.timer = setTimeout(() => this.p.kill(), 20000);
  }
  send(msg) {
    this.p.stdin.write(JSON.stringify(msg) + "\n");
  }
  async recv() {
    const line = this.lines.length ? this.lines.shift() : await new Promise((r) => this.waiters.push(r));
    return JSON.parse(line);
  }
  call(method, params) {
    this.send({ jsonrpc: "2.0", id: ++this.next, method, params });
  }
  /** Closes stdin like Workboard does; resolves to { code, rest (unread stdout lines), stderr }. */
  async finish() {
    this.p.stdin.end();
    const code = await this.exited;
    clearTimeout(this.timer);
    await new Promise((r) => setImmediate(r));
    return { code, rest: this.lines.join("\n"), stderr: this.stderr };
  }
}

async function started() {
  const p = new Proc();
  p.call("initialize", INIT);
  assert.deepEqual(await p.recv(), { jsonrpc: "2.0", id: 1, result: {} });
  return p;
}

test("open with interleaved host requests", async () => {
  const p = await started();
  p.call("action/open", { action: "greet", context: CTX });
  let req = await p.recv();
  assert.equal(req.method, "host/item");
  assert.deepEqual(req.params, { name: "wi1-login" });
  p.send({ jsonrpc: "2.0", id: req.id, result: { name: "wi1-login", title: "Login" } });
  req = await p.recv();
  assert.equal(req.method, "host/secret/get");
  p.send({ jsonrpc: "2.0", id: req.id, error: { code: -32000, message: "plugin.denied" } });
  const out = await p.recv();
  assert.equal(out.id, 2);
  const step = out.result;
  assert.equal(step.type, "form");
  assert.equal(step.title, "Login · es");
  assert.deepEqual(step.state, { denied: "plugin.denied", placement: "item", dir: HERE });
  assert.deepEqual(step.fields[0], { id: "name", type: "text", label: "Name", required: true });
  assert.deepEqual(step.fields[1].options, [
    { value: "a", label: "A" },
    { value: "b", label: "b" },
  ]);
  assert.equal(step.fields[1].refresh, true);
  assert.deepEqual(step.fields[2], { type: "info", text: "Some help" });
  assert.deepEqual(step.buttons, [
    { id: "more", label: "More" },
    { id: "greet", label: "Greet", primary: true, confirm: "Sure?" },
  ]);
  const { code, rest, stderr } = await p.finish();
  assert.equal(code, 0);
  assert.equal(rest, "");
  assert.match(stderr, /stray log goes to stderr/);
});

test("submit and UTF-8", async () => {
  const p = await started();
  p.call("action/submit", {
    action: "greet",
    context: CTX,
    button: "greet",
    values: { name: "Ñandú" },
    state: { k: 1 },
  });
  const req = await p.recv();
  assert.equal(req.method, "host/updateItem");
  assert.deepEqual(req.params, { name: "wi1-login", fields: { notes: "greeted Ñandú" } });
  p.send({ jsonrpc: "2.0", id: req.id, result: { name: "wi1-login" } });
  assert.deepEqual((await p.recv()).result, {
    type: "result",
    title: "Hello",
    ok: true,
    message: "¡Hola, Ñandú!",
    steps: [{ label: "Saved", ok: true }],
    links: [{ label: "Open", url: "https://example.com" }],
    state: { k: 1 },
  });
  assert.equal((await p.finish()).code, 0);
});

test("session and close steps", async () => {
  let p = await started();
  p.call("action/submit", { action: "greet", context: CTX, button: "session", values: {}, state: null });
  assert.deepEqual((await p.recv()).result, { type: "session", items: ["wi1-login"], mode: "plan", prompt: "Hi" });
  await p.finish();
  p = await started();
  p.call("action/submit", { action: "greet", context: CTX, button: "close", values: null });
  assert.deepEqual((await p.recv()).result, { type: "close", message: "Saved" });
  await p.finish();
});

test("errors", async () => {
  let p = await started();
  p.call("action/open", { action: "boom", context: CTX });
  assert.equal((await p.recv()).error.message, "TypeError: boom");
  const { rest, stderr } = await p.finish();
  assert.equal(rest, "");
  assert.match(stderr, /TypeError: boom\r?\n\s+at /);
  p = await started();
  p.call("action/open", { action: "nope", context: CTX });
  assert.match((await p.recv()).error.message, /unknown action/);
  await p.finish();
  p = await started();
  p.call("action/frobnicate", {});
  const err = (await p.recv()).error;
  assert.equal(err.code, -32601);
  assert.match(err.message, /action\/frobnicate/);
  await p.finish();
});

test("unhandled host errors", async () => {
  const p = await started();
  p.call("action/open", { action: "hosterr", context: CTX });
  const req = await p.recv();
  assert.equal(req.method, "host/readFile");
  p.send({ jsonrpc: "2.0", id: req.id, error: { code: -32000, message: "plugin.denied" } });
  assert.equal((await p.recv()).error.message, "host/readFile failed: plugin.denied");
  await p.finish();
});

test("a newer protocol is rejected", async () => {
  const p = new Proc();
  p.call("initialize", { ...INIT, protocol: 2 });
  assert.match((await p.recv()).error.message, /protocol 2 is newer/);
  assert.equal((await p.finish()).code, 0);
});

test("exits when stdin closes", async () => {
  const p = new Proc();
  const { code, rest } = await p.finish();
  assert.equal(code, 0);
  assert.equal(rest, "");
});

async function tracker(method, extra = {}, secret = "s3cret-key") {
  const p = await started();
  p.call(method, { kind: "fake", connection: CONN, secret, ...extra });
  const out = await p.recv();
  const { code, rest, stderr } = await p.finish();
  assert.equal(code, 0);
  assert.equal(rest, "");
  assert.ok(!stderr.includes("s3cret-key"));
  return out;
}

test("tracker methods", async () => {
  assert.deepEqual((await tracker("tracker/connect")).result, { org: "acme", project: "BOARD" });
  assert.equal((await tracker("tracker/states")).result[0].suggested, "active");
  assert.deepEqual((await tracker("tracker/workItems", { ids: ["1", "2"], request: { text: true } })).result, {
    1: { title: "T1", text: true },
    2: { title: "T2", text: true },
  });
  const assigned = await tracker("tracker/assignedToMe", {
    dueField: "Due",
    start: "2026-01-01",
    end: "2026-02-01",
    exclude: ["Done"],
  });
  assert.deepEqual(assigned.result, ["Due", "2026-01-01", "2026-02-01", ["Done"]]);
  assert.deepEqual((await tracker("tracker/detail", { id: "7" })).result, { id: "7", title: "Detail" });
  assert.deepEqual((await tracker("tracker/setState", { id: "7", state: "Done" })).result, {
    state: "Done",
    changed: "2026-01-01",
  });
});

test("tracker errors", async () => {
  assert.equal((await tracker("tracker/connect", {}, "wrong")).error.message, "tracker.auth");
  assert.equal((await tracker("tracker/detail", { id: "404" })).error.message, "tracker.notFound");
  // The credential never comes back in an error.
  assert.equal((await tracker("tracker/fields")).error.message, "Error: leaked ***");
  assert.match((await tracker("tracker/connect", { kind: "other" })).error.message, /unknown tracker kind/);
});

test("builders", () => {
  assert.deepEqual(wb.textarea("n", "Notes", { rows: 6 }), { id: "n", type: "textarea", label: "Notes", rows: 6 });
  assert.deepEqual(wb.password("t", "Token", { placeholder: "saved" }), {
    id: "t",
    type: "password",
    label: "Token",
    placeholder: "saved",
  });
  assert.deepEqual(wb.checkbox("c", "Link", { value: true }), { id: "c", type: "checkbox", label: "Link", value: true });
  assert.deepEqual(wb.checkbox("c", "Link"), { id: "c", type: "checkbox", label: "Link", value: false });
  assert.deepEqual(wb.number("n", "Count", { value: 0, wide: true, hint: "h" }), {
    id: "n",
    type: "number",
    label: "Count",
    value: 0,
    wide: true,
    hint: "h",
  });
  assert.deepEqual(wb.button("m", "More", { validate: false }), { id: "m", label: "More", validate: false });
  assert.deepEqual(wb.form("T", [], { intro: "i", error: "e" }), {
    type: "form",
    title: "T",
    intro: "i",
    error: "e",
    fields: [],
    buttons: [],
  });
  assert.deepEqual(
    wb.result("T", undefined, { ok: false, steps: [["B", false, "x"]], buttons: [["retry", "Retry"]] }),
    {
      type: "result",
      title: "T",
      ok: false,
      steps: [{ label: "B", ok: false, error: "x" }],
      buttons: [{ id: "retry", label: "Retry" }],
    },
  );
  assert.deepEqual(wb.close(), { type: "close" });
  const e = new wb.TrackerError("tracker.auth", "bad token");
  assert.deepEqual([e.code, e.detail, e.message], ["tracker.auth", "bad token", "tracker.auth"]);
});
