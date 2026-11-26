"""Tests for workboard_plugin: the fixture plugin runs as a subprocess speaking the protocol, like
Workboard runs it. Run with ``python -m unittest discover -s sdk/python/tests``."""
import json
import os
import subprocess
import sys
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import workboard_plugin as wb  # noqa: E402

FIXTURE = os.path.join(HERE, "fixture_plugin.py")
INIT = {"protocol": 1, "host": {"name": "workboard", "version": "0.1.0"}, "lang": "es",
        "plugin": {"id": "fixture", "dir": HERE}}
CTX = {"lang": "es", "root": "/r", "placement": "item", "items": [{"name": "wi1-login", "kind": "item"}]}
CONN = {"org": "acme/", "project": "board"}


class Proc:
    """The fixture plugin as Workboard sees it."""

    def __init__(self):
        self.p = subprocess.Popen([sys.executable, FIXTURE], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE)
        self.timer = threading.Timer(20, self.p.kill)
        self.timer.start()
        self.next = 0

    def send(self, msg):
        self.p.stdin.write((json.dumps(msg, ensure_ascii=False) + "\n").encode("utf-8"))
        self.p.stdin.flush()

    def recv(self):
        line = self.p.stdout.readline()
        if not line:
            raise AssertionError("plugin closed stdout")
        return json.loads(line.decode("utf-8"))

    def call(self, method, params):
        self.next += 1
        self.send({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params})
        return self.next

    def finish(self):
        """Closes stdin like Workboard does; returns (exit code, rest of stdout, stderr)."""
        self.p.stdin.close()
        out = self.p.stdout.read()
        err = self.p.stderr.read()
        code = self.p.wait(10)
        self.timer.cancel()
        self.p.stdout.close()
        self.p.stderr.close()
        return code, out.decode("utf-8"), err.decode("utf-8", "replace")


class ProtocolTest(unittest.TestCase):
    def start(self):
        p = Proc()
        p.call("initialize", INIT)
        self.assertEqual(p.recv(), {"jsonrpc": "2.0", "id": 1, "result": {}})
        return p

    def test_open_with_interleaved_host_requests(self):
        p = self.start()
        p.call("action/open", {"action": "greet", "context": CTX})
        req = p.recv()
        self.assertEqual(req["method"], "host/item")
        self.assertEqual(req["params"], {"name": "wi1-login"})
        p.send({"jsonrpc": "2.0", "id": req["id"], "result": {"name": "wi1-login", "title": "Login"}})
        req = p.recv()
        self.assertEqual(req["method"], "host/secret/get")
        p.send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32000, "message": "plugin.denied"}})
        out = p.recv()
        self.assertEqual(out["id"], 2)
        step = out["result"]
        self.assertEqual(step["type"], "form")
        self.assertEqual(step["title"], "Login · es")
        self.assertEqual(step["state"], {"denied": "plugin.denied", "placement": "item", "dir": HERE})
        self.assertEqual(step["fields"][0], {"id": "name", "type": "text", "label": "Name", "required": True})
        self.assertEqual(step["fields"][1]["options"], [{"value": "a", "label": "A"}, {"value": "b", "label": "b"}])
        self.assertTrue(step["fields"][1]["refresh"])
        self.assertEqual(step["fields"][2], {"type": "info", "text": "Some help"})
        self.assertEqual(step["buttons"], [{"id": "more", "label": "More"},
                                           {"id": "greet", "label": "Greet", "primary": True, "confirm": "Sure?"}])
        code, rest, err = p.finish()
        self.assertEqual(code, 0)
        self.assertEqual(rest, "")
        self.assertIn("stray print goes to stderr", err)

    def test_submit_and_utf8(self):
        p = self.start()
        p.call("action/submit", {"action": "greet", "context": CTX, "button": "greet",
                                 "values": {"name": "Ñandú"}, "state": {"k": 1}})
        req = p.recv()
        self.assertEqual(req["method"], "host/updateItem")
        self.assertEqual(req["params"], {"name": "wi1-login", "fields": {"notes": "greeted Ñandú"}})
        p.send({"jsonrpc": "2.0", "id": req["id"], "result": {"name": "wi1-login"}})
        step = p.recv()["result"]
        self.assertEqual(step, {"type": "result", "title": "Hello", "ok": True, "message": "¡Hola, Ñandú!",
                                "state": {"k": 1}, "steps": [{"label": "Saved", "ok": True}],
                                "links": [{"label": "Open", "url": "https://example.com"}]})
        self.assertEqual(p.finish()[0], 0)

    def test_session_and_close_steps(self):
        p = self.start()
        p.call("action/submit", {"action": "greet", "context": CTX, "button": "session", "values": {}, "state": None})
        self.assertEqual(p.recv()["result"], {"type": "session", "items": ["wi1-login"], "mode": "plan", "prompt": "Hi"})
        p.finish()
        p = self.start()
        p.call("action/submit", {"action": "greet", "context": CTX, "button": "close", "values": None})
        self.assertEqual(p.recv()["result"], {"type": "close", "message": "Saved"})
        p.finish()

    def test_errors(self):
        p = self.start()
        p.call("action/open", {"action": "boom", "context": CTX})
        out = p.recv()
        self.assertEqual(out["error"]["message"], "ValueError: boom")
        code, rest, err = p.finish()
        self.assertEqual(rest, "")
        self.assertIn("Traceback", err)
        p = self.start()
        p.call("action/open", {"action": "nope", "context": CTX})
        self.assertIn("unknown action", p.recv()["error"]["message"])
        p.finish()
        p = self.start()
        p.call("action/frobnicate", {})
        err = p.recv()["error"]
        self.assertEqual(err["code"], -32601)
        self.assertIn("action/frobnicate", err["message"])
        p.finish()

    def test_host_error_unhandled(self):
        p = self.start()
        p.call("action/open", {"action": "hosterr", "context": CTX})
        req = p.recv()
        self.assertEqual(req["method"], "host/readFile")
        p.send({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32000, "message": "plugin.denied"}})
        self.assertEqual(p.recv()["error"]["message"], "host/readFile failed: plugin.denied")
        p.finish()

    def test_newer_protocol_is_rejected(self):
        p = Proc()
        p.call("initialize", dict(INIT, protocol=2))
        err = p.recv()["error"]
        self.assertIn("protocol 2 is newer", err["message"])
        self.assertEqual(p.finish()[0], 0)

    def test_exits_when_stdin_closes(self):
        p = Proc()
        self.assertEqual(p.finish()[:2], (0, ""))


class TrackerTest(unittest.TestCase):
    def call(self, method, extra=None, secret="s3cret-key"):
        p = Proc()
        p.call("initialize", INIT)
        p.recv()
        params = {"kind": "fake", "connection": CONN, "secret": secret}
        params.update(extra or {})
        p.call(method, params)
        out = p.recv()
        code, rest, err = p.finish()
        self.assertEqual((code, rest), (0, ""))
        self.assertNotIn("s3cret-key", err)
        return out

    def test_methods(self):
        self.assertEqual(self.call("tracker/connect")["result"], {"org": "acme", "project": "BOARD"})
        self.assertEqual(self.call("tracker/states")["result"][0]["suggested"], "active")
        self.assertEqual(self.call("tracker/workItems", {"ids": ["1", "2"], "request": {"text": True}})["result"],
                         {"1": {"title": "T1", "text": True}, "2": {"title": "T2", "text": True}})
        self.assertEqual(
            self.call("tracker/assignedToMe", {"dueField": "Due", "start": "2026-01-01", "end": "2026-02-01",
                                               "exclude": ["Done"]})["result"],
            ["Due", "2026-01-01", "2026-02-01", ["Done"]])
        self.assertEqual(self.call("tracker/detail", {"id": "7"})["result"], {"id": "7", "title": "Detail"})
        self.assertEqual(self.call("tracker/setState", {"id": "7", "state": "Done"})["result"],
                         {"state": "Done", "changed": "2026-01-01"})

    def test_errors(self):
        err = self.call("tracker/connect", secret="wrong")["error"]
        self.assertEqual(err["message"], "tracker.auth")
        self.assertEqual(self.call("tracker/detail", {"id": "404"})["error"]["message"], "tracker.notFound")
        # The credential never comes back in an error.
        self.assertEqual(self.call("tracker/fields")["error"]["message"], "RuntimeError: leaked ***")
        err = self.call("tracker/connect", {"kind": "other"})["error"]
        self.assertIn("unknown tracker kind", err["message"])


class BuildersTest(unittest.TestCase):
    def test_fields(self):
        self.assertEqual(wb.textarea("n", "Notes", rows=6), {"id": "n", "type": "textarea", "label": "Notes", "rows": 6})
        self.assertEqual(wb.password("t", "Token", placeholder="saved"),
                         {"id": "t", "type": "password", "label": "Token", "placeholder": "saved"})
        self.assertEqual(wb.checkbox("c", "Link", value=True), {"id": "c", "type": "checkbox", "label": "Link", "value": True})
        self.assertEqual(wb.number("n", "Count", value=3, wide=True, hint="h"),
                         {"id": "n", "type": "number", "label": "Count", "value": 3, "wide": True, "hint": "h"})
        self.assertEqual(wb.button("m", "More", validate=False), {"id": "m", "label": "More", "validate": False})

    def test_steps(self):
        self.assertEqual(wb.form("T", [], intro="i", error="e"),
                         {"type": "form", "title": "T", "intro": "i", "error": "e", "fields": [], "buttons": []})
        self.assertEqual(wb.result("T", ok=False, steps=[("B", False, "x")], buttons=[("retry", "Retry")]),
                         {"type": "result", "title": "T", "ok": False,
                          "steps": [{"label": "B", "ok": False, "error": "x"}],
                          "buttons": [{"id": "retry", "label": "Retry"}]})
        self.assertEqual(wb.close(), {"type": "close"})

    def test_tracker_error(self):
        e = wb.TrackerError("tracker.auth", "bad token")
        self.assertEqual((e.code, e.message, str(e)), ("tracker.auth", "bad token", "tracker.auth"))
        self.assertEqual(str(wb.TrackerError(message="plain")), "plain")


if __name__ == "__main__":
    unittest.main()
