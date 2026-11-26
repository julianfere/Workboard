import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.dirname(HERE))

import trello_api as t  # noqa: E402
from fake_trello import (BOARD_ID, DEADLINE, HOURS, KEY, SECRET, SHORT, SIZE, TOKEN,  # noqa: E402
                         FakeTrello)

CONN = {"org": "", "project": SHORT}


def board(conn=CONN, secret=SECRET, fake=None):
    fake = fake or FakeTrello()
    return t.Board(conn, secret, fake), fake


class SecretTests(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(t.parse_secret(" abc : def "), ("abc", "def"))
        self.assertEqual(t.parse_secret("abc:de:f"), ("abc", "de:f"))
        for bad in ["", None, "abc", ":def", "abc:"]:
            with self.assertRaises(t.TrelloError) as e:
                t.parse_secret(bad)
            self.assertEqual(e.exception.code, "tracker.auth")

    def test_redact(self):
        text = "key=%s&token=%s whole=%s" % (KEY, TOKEN, SECRET)
        out = t.redact(text, SECRET)
        self.assertNotIn(KEY, out)
        self.assertNotIn(TOKEN, out)
        self.assertEqual(t.redact("x", ""), "x")

    def test_credential_only_in_header(self):
        b, fake = board()
        b.states()
        for url in fake.urls:
            self.assertNotIn(KEY, url)
            self.assertNotIn(TOKEN, url)


class NormalizeTests(unittest.TestCase):
    def test_board(self):
        self.assertEqual(t.normalize_board("AbCd1234"), "AbCd1234")
        self.assertEqual(t.normalize_board(" %s " % BOARD_ID), BOARD_ID)
        self.assertEqual(t.normalize_board("https://trello.com/b/AbCd1234/my-board"), "AbCd1234")
        self.assertEqual(t.normalize_board("trello.com/b/AbCd1234"), "AbCd1234")
        for bad in ["", "abc", "https://example.com/b/AbCd1234", "AbCd 1234", "../x/AbCd1234"]:
            with self.assertRaises(t.TrelloError) as e:
                t.normalize_board(bad)
            self.assertEqual(e.exception.code, "tracker.notFound")

    def test_workspace(self):
        self.assertEqual(t.normalize_workspace(""), "")
        self.assertEqual(t.normalize_workspace("my-team"), "my-team")
        self.assertEqual(t.normalize_workspace("https://trello.com/w/my-team/home"), "my-team")
        with self.assertRaises(t.TrelloError):
            t.normalize_workspace("my team")

    def test_suggest(self):
        cases = {"Backlog": "backlog", "To Do": "backlog", "TODO": "backlog", "Por hacer": "backlog",
                 "Doing": "active", "In Progress": "active", "En curso": "active",
                 "Review": "review", "QA": "review", "Testing": "review", "En revisión": "review",
                 "Done": "done", "Hecho": "done", "Terminado ✓": "done", "Cancelled": "cancelled",
                 "Misc": None, "Undone ideas?": "backlog", "": None, "Quality": None}
        for name, want in cases.items():
            self.assertEqual(t.suggest(name), want, name)

    def test_checklist_markdown(self):
        self.assertEqual(t.checklist_markdown([]), "")
        one = [{"name": "C", "checkItems": [{"name": "b", "state": "complete", "pos": 2},
                                             {"name": "a", "state": "incomplete", "pos": 1}]}]
        self.assertEqual(t.checklist_markdown(one), "- [ ] a\n- [x] b")


class ConnectTests(unittest.TestCase):
    def test_board_url_becomes_short_link(self):
        b, fake = board({"org": "", "project": "https://trello.com/b/%s/team" % BOARD_ID})
        self.assertEqual(b.connect(), {"org": "", "project": SHORT})

    def test_workspace_checked(self):
        b, _ = board({"org": "https://trello.com/w/my-team", "project": SHORT})
        self.assertEqual(b.connect(), {"org": "my-team", "project": SHORT})
        b, _ = board({"org": "other", "project": SHORT})
        with self.assertRaises(t.TrelloError) as e:
            b.connect()
        self.assertEqual(e.exception.code, "tracker.notFound")
        b, _ = board({"org": "nobody", "project": SHORT})
        with self.assertRaises(t.TrelloError) as e:
            b.connect()
        self.assertIn("Workspace", e.exception.message)

    def test_missing_board(self):
        for project in ["ZzZz9999", "0" * 24]:
            b, _ = board({"org": "", "project": project})
            with self.assertRaises(t.TrelloError) as e:
                b.connect()
            self.assertEqual(e.exception.code, "tracker.notFound", project)


class ErrorTests(unittest.TestCase):
    def test_bad_credential(self):
        b, _ = board(secret="wrong:creds")
        with self.assertRaises(t.TrelloError) as e:
            b.states()
        self.assertEqual(e.exception.code, "tracker.auth")

    def test_invalid_key_and_redaction(self):
        fake = FakeTrello()
        fake.fail = (401, "invalid key %s for token %s" % (KEY, TOKEN))
        b, _ = board(fake=fake)
        with self.assertRaises(t.TrelloError) as e:
            b.states()
        self.assertEqual(e.exception.code, "tracker.auth")
        self.assertNotIn(KEY, e.exception.message)
        self.assertNotIn(TOKEN, e.exception.message)
        self.assertNotIn(TOKEN, str(e.exception))

    def test_other_status_redacted(self):
        fake = FakeTrello()
        fake.fail = (500, '{"message": "boom with %s"}' % SECRET)
        b, _ = board(fake=fake)
        with self.assertRaises(t.TrelloError) as e:
            b.fields()
        self.assertEqual(e.exception.code, "tracker.error")
        self.assertIn("500", e.exception.message)
        self.assertIn("boom", e.exception.message)
        self.assertNotIn(KEY, e.exception.message)

    def test_rate_limit_and_not_json(self):
        fake = FakeTrello()
        fake.fail = (429, "slow down")
        with self.assertRaises(t.TrelloError) as e:
            board(fake=fake)[0].states()
        self.assertIn("rate limit", e.exception.message)
        fake.fail = (200, "<html>")
        with self.assertRaises(t.TrelloError):
            board(fake=fake)[0].states()

    def test_network_error_redacted(self):
        fake = FakeTrello()
        fake.raise_exc = OSError("connection reset while sending %s" % TOKEN)
        with self.assertRaises(t.TrelloError) as e:
            board(fake=fake)[0].states()
        self.assertEqual(e.exception.code, "tracker.error")
        self.assertNotIn(TOKEN, e.exception.message)


class ReadTests(unittest.TestCase):
    def test_states(self):
        b, _ = board()
        states = b.states()
        self.assertEqual([s["name"] for s in states], ["Backlog", "Doing", "En revisión", "Done", "Misc"])
        self.assertEqual([s["suggested"] for s in states], ["backlog", "active", "review", "done", None])
        self.assertEqual(states[1]["color"], "61BD4F")
        self.assertEqual(states[0]["color"], "")
        self.assertEqual(states[0]["types"], [])

    def test_fields(self):
        b, _ = board()
        fields = b.fields()
        self.assertEqual(fields[0], {"ref": "due", "name": "Due date", "type": "date", "role": "due"})
        by_ref = {f["ref"]: f for f in fields}
        self.assertEqual(by_ref[HOURS]["type"], "number")
        self.assertEqual(by_ref[HOURS]["role"], "hours")
        self.assertEqual(by_ref[DEADLINE]["type"], "date")
        self.assertNotIn("role", by_ref[DEADLINE])
        self.assertEqual(by_ref[SIZE]["type"], "list")

    def test_work_items_basic(self):
        b, fake = board()
        items = b.work_items(["1", "5", "99", "abc", "2"],
                             {"priority": True, "due": None, "hours": None, "effort": True, "text": False,
                              "extra": []})
        self.assertEqual(sorted(items), ["1", "2", "5"])
        one = items["1"]
        self.assertEqual(one["state"], "Doing")
        self.assertEqual(one["title"], "Fix login")
        self.assertEqual(one["type"], "Bug")
        self.assertEqual(one["assigned"], "Ana Pérez, Bo Li")
        self.assertEqual(one["changed"], "2026-11-10T10:00:00.000Z")
        self.assertEqual(one["due"], "2026-11-20T15:00:00.000Z")
        self.assertIsNone(one["priority"])
        self.assertIsNone(one["estimate"])
        self.assertIsNone(one["completed"])
        self.assertNotIn("description", one)
        self.assertEqual(items["2"]["type"], "")
        self.assertEqual(items["5"]["state"], "Old")  # archived card in an archived list
        # one batch: the board's cards once, not one request per id
        card_calls = [c for c in fake.calls if c[1].startswith("boards/%s/cards" % SHORT)]
        self.assertEqual(len(card_calls), 1)
        self.assertFalse(any(c[1] == "boards/%s/customFields" % SHORT for c in fake.calls))

    def test_work_items_no_valid_ids(self):
        b, fake = board()
        self.assertEqual(b.work_items(["abc"], {}), {})
        self.assertEqual(fake.calls, [])

    def test_work_items_custom_fields_and_text(self):
        b, _ = board()
        items = b.work_items(["1", "7"], {"due": DEADLINE, "hours": HOURS, "text": True,
                                          "extra": [HOURS, SIZE, "due", "missing"]})
        one = items["1"]
        self.assertEqual(one["due"], "2026-12-01T00:00:00.000Z")
        self.assertEqual(one["estimate"], 6.5)
        self.assertEqual(one["fields"], {HOURS: 6.5, SIZE: "L", "due": "2026-11-20T15:00:00.000Z",
                                         "missing": None})
        self.assertEqual(one["description"], "Users **cannot** log in.")
        self.assertEqual(one["criteria"], "- [ ] Error shown\n- [x] Login works")
        self.assertEqual(items["7"]["criteria"], "**A**\n- [ ] a1\n\n**B**\n- [x] b1")
        self.assertEqual(items["7"]["due"], "")
        self.assertIsNone(items["7"]["estimate"])

    def test_assigned_to_me(self):
        b, _ = board()
        self.assertEqual(b.assigned_to_me("due", "2026-11-01", "2026-12-01", []), ["2", "3", "1"])
        self.assertEqual(b.assigned_to_me("due", "2026-11-01", "2026-12-01", ["Done"]), ["2", "1"])
        self.assertEqual(b.assigned_to_me("due", "2026-11-06", "2026-11-20", []), ["3"])
        self.assertEqual(b.assigned_to_me(DEADLINE, "2026-12-01", "2026-12-02", []), ["1"])

    def test_detail(self):
        b, _ = board()
        self.assertEqual(b.detail("1"), {"id": "1", "title": "Fix login", "type": "Bug", "state": "Doing",
                                         "description": "Users **cannot** log in.",
                                         "criteria": "- [ ] Error shown\n- [x] Login works"})
        for bad in ["99", "x1"]:
            with self.assertRaises(t.TrelloError) as e:
                b.detail(bad)
            self.assertEqual(e.exception.code, "tracker.notFound")


class WriteTests(unittest.TestCase):
    def test_set_state_moves_card(self):
        b, fake = board()
        self.assertEqual(b.set_state("2", "Doing"),
                         {"state": "Doing", "changed": "2026-11-26T12:00:00.000Z"})
        put = [c for c in fake.calls if c[0] == "PUT"]
        self.assertEqual(put, [("PUT", "cards/c2", {"idList": "L2"})])
        self.assertEqual(b.work_items(["2"], {})["2"]["state"], "Doing")

    def test_set_state_errors(self):
        b, fake = board()
        with self.assertRaises(t.TrelloError) as e:
            b.set_state("2", "Old")  # archived list
        self.assertIn("Old", e.exception.message)
        with self.assertRaises(t.TrelloError) as e:
            b.set_state("2", "Nope")
        self.assertEqual(e.exception.code, "tracker.error")
        with self.assertRaises(t.TrelloError) as e:
            b.set_state("99", "Doing")
        self.assertEqual(e.exception.code, "tracker.notFound")
        self.assertFalse(any(c[0] == "PUT" for c in fake.calls))


if __name__ == "__main__":
    unittest.main()
