import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PLUGIN_DIR = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, PLUGIN_DIR)

import plugin  # noqa: E402
from fake_trello import KEY, SECRET, SHORT, TOKEN, FakeTrello  # noqa: E402

CONN = {"org": "", "project": SHORT}


def call(method, secret=SECRET, **extra):
    params = {"kind": "trello", "connection": CONN, "secret": secret}
    params.update(extra)
    return plugin.plugin.handle(method, params)


class InProcessTests(unittest.TestCase):
    def setUp(self):
        plugin.HTTP = FakeTrello()

    def tearDown(self):
        plugin.HTTP = None

    def test_every_method(self):
        self.assertEqual(call("tracker/connect")["project"], SHORT)
        self.assertEqual(len(call("tracker/states")), 5)
        self.assertEqual(call("tracker/fields")[0]["ref"], "due")
        items = call("tracker/workItems", ids=["1", "2"], request={"text": True})
        self.assertEqual(sorted(items), ["1", "2"])
        self.assertEqual(call("tracker/assignedToMe", dueField="due", start="2026-11-01",
                              end="2026-12-01", exclude=["Done"]), ["2", "1"])
        self.assertEqual(call("tracker/detail", id="1")["title"], "Fix login")
        self.assertEqual(call("tracker/setState", id="1", state="Done")["state"], "Done")

    def test_error_codes(self):
        with self.assertRaises(plugin.TrackerError) as e:
            call("tracker/states", secret="bad:creds")
        self.assertEqual(e.exception.code, "tracker.auth")
        with self.assertRaises(plugin.TrackerError) as e:
            call("tracker/detail", id="99")
        self.assertEqual(e.exception.code, "tracker.notFound")
        with self.assertRaises(plugin.TrackerError) as e:
            call("tracker/setState", id="1", state="Nope")
        self.assertIsNone(e.exception.code)
        self.assertIn("Nope", e.exception.message)

    def test_error_answer_has_no_secret(self):
        fake = FakeTrello()
        fake.fail = (401, "invalid key %s token %s" % (KEY, TOKEN))
        plugin.HTTP = fake
        with self.assertRaises(plugin.TrackerError) as e:
            call("tracker/states")
        self.assertNotIn(KEY, e.exception.message)
        self.assertNotIn(TOKEN, e.exception.message)

    def test_default_transport_is_urllib(self):
        plugin.HTTP = None
        self.assertIs(plugin.trello_api.Board(CONN, SECRET, plugin.HTTP).client.http,
                      plugin.trello_api.urllib_http)


class ProtocolTests(unittest.TestCase):
    """Runs the plugin as a separate process and speaks JSON-RPC over its stdio."""

    def run_plugin(self, messages):
        lines = "".join(json.dumps(m) + "\n" for m in messages).encode("utf-8")
        proc = subprocess.run([sys.executable, os.path.join(HERE, "run_plugin.py")], input=lines,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=PLUGIN_DIR, timeout=30)
        self.assertEqual(proc.returncode, 0, proc.stderr.decode("utf-8", "replace"))
        return [json.loads(l) for l in proc.stdout.decode("utf-8").splitlines() if l.strip()]

    def test_initialize_and_states(self):
        out = self.run_plugin([
            {"jsonrpc": "2.0", "id": 1, "method": "initialize",
             "params": {"protocol": 1, "host": {"name": "workboard", "version": "0.1.0"}, "lang": "es",
                        "plugin": {"id": "trello", "dir": PLUGIN_DIR}}},
            {"jsonrpc": "2.0", "id": 2, "method": "tracker/states",
             "params": {"kind": "trello", "connection": CONN, "secret": SECRET}},
            {"jsonrpc": "2.0", "id": 3, "method": "tracker/states",
             "params": {"kind": "trello", "connection": CONN, "secret": "wrong:%s" % TOKEN}},
        ])
        self.assertEqual(out[0], {"jsonrpc": "2.0", "id": 1, "result": {}})
        self.assertEqual(out[1]["id"], 2)
        self.assertEqual([s["name"] for s in out[1]["result"]],
                         ["Backlog", "Doing", "En revisión", "Done", "Misc"])
        self.assertEqual(out[2]["id"], 3)
        self.assertEqual(out[2]["error"]["message"], "tracker.auth")
        self.assertNotIn(TOKEN, json.dumps(out[2]))

    def test_plugin_py_runs_without_the_fake(self):
        # plugin.py itself starts, answers initialize and exits when stdin closes (no network used).
        proc = subprocess.run([sys.executable, "plugin.py"], cwd=PLUGIN_DIR, timeout=30,
                              input=b'{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":1}}\n',
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(json.loads(proc.stdout), {"jsonrpc": "2.0", "id": 1, "result": {}})


if __name__ == "__main__":
    unittest.main()
