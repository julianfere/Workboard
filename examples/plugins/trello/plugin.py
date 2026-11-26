"""Trello tracker plugin for Workboard (see README.md and docs/plugins.md). Python 3.8+, stdlib only.

Thin entry point: the protocol is handled by the SDK (`workboard_plugin.py`, copied next to this file)
and the Trello logic lives in `trello_api.py`.
"""
import trello_api
from workboard_plugin import Plugin, TrackerError

plugin = Plugin()

# Transport used by `trello_api` (None = urllib). Only `tests/run_plugin.py`, which imports this module
# instead of running it, replaces it; nothing in the environment or the protocol can change it.
HTTP = None

# Codes Workboard tells apart; any other error goes back as a plain message.
CODES = ("tracker.auth", "tracker.notFound")


def _call(conn, secret, fn):
    try:
        return fn(trello_api.Board(conn, secret, HTTP))
    except trello_api.TrelloError as e:
        code = e.code if e.code in CODES else None
        raise TrackerError(code, trello_api.redact(e.message, secret))


@plugin.tracker("trello")
class Trello:
    def connect(self, conn, secret):
        return _call(conn, secret, lambda b: b.connect())

    def states(self, conn, secret):
        return _call(conn, secret, lambda b: b.states())

    def fields(self, conn, secret):
        return _call(conn, secret, lambda b: b.fields())

    def work_items(self, conn, secret, ids, request):
        return _call(conn, secret, lambda b: b.work_items(ids, request))

    def assigned_to_me(self, conn, secret, due_field, start, end, exclude):
        return _call(conn, secret, lambda b: b.assigned_to_me(due_field, start, end, exclude))

    def detail(self, conn, secret, id):
        return _call(conn, secret, lambda b: b.detail(id))

    def set_state(self, conn, secret, id, state):
        return _call(conn, secret, lambda b: b.set_state(id, state))


def main():
    plugin.run()


if __name__ == "__main__":
    main()
