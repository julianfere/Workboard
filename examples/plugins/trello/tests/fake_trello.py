"""A fake Trello API for the tests: an `http(method, url, headers, body, timeout)` over fixed data."""
import json
import urllib.parse

KEY = "k3y0123456789abcdef"
TOKEN = "t0ken0123456789abcdefTOKEN"
SECRET = KEY + ":" + TOKEN
AUTH = 'OAuth oauth_consumer_key="%s", oauth_token="%s"' % (KEY, TOKEN)

BOARD_ID = "5f1e2d3c4b5a69788796a5b4"
SHORT = "AbCd1234"
ME = "me0000000000000000000001"
OTHER = "me0000000000000000000002"
HOURS = "cf00000000000000000000h1"
DEADLINE = "cf00000000000000000000d1"
SIZE = "cf00000000000000000000s1"

LISTS = [
    {"id": "L1", "name": "Backlog", "closed": False, "color": None, "pos": 1},
    {"id": "L2", "name": "Doing", "closed": False, "color": "green", "pos": 2},
    {"id": "L3", "name": "En revisión", "closed": False, "color": None, "pos": 3},
    {"id": "L4", "name": "Done", "closed": False, "color": None, "pos": 4},
    {"id": "L5", "name": "Misc", "closed": False, "color": None, "pos": 5},
    {"id": "L9", "name": "Old", "closed": True, "color": None, "pos": 9},
]

CUSTOM_FIELDS = [
    {"id": HOURS, "name": "Horas estimadas", "type": "number"},
    {"id": DEADLINE, "name": "Deadline", "type": "date"},
    {"id": SIZE, "name": "Size", "type": "list",
     "options": [{"id": "o1", "value": {"text": "S"}}, {"id": "o2", "value": {"text": "L"}}]},
]

CARDS = [
    {"id": "c1", "idShort": 1, "name": "Fix login", "desc": "Users **cannot** log in.",
     "due": "2026-11-20T15:00:00.000Z", "dateLastActivity": "2026-11-10T10:00:00.000Z", "idList": "L2",
     "idMembers": [ME], "members": [{"id": ME, "fullName": "Ana Pérez"}, {"id": OTHER, "fullName": "Bo Li"}],
     "labels": [{"name": "", "color": "red"}, {"name": "Bug", "color": "red"}], "closed": False,
     "customFieldItems": [{"idCustomField": HOURS, "value": {"number": "6.5"}},
                          {"idCustomField": DEADLINE, "value": {"date": "2026-12-01T00:00:00.000Z"}},
                          {"idCustomField": SIZE, "idValue": "o2"}],
     "checklists": [{"name": "Criteria", "pos": 1, "checkItems": [
         {"name": "Login works", "state": "complete", "pos": 2},
         {"name": "Error shown", "state": "incomplete", "pos": 1}]}]},
    {"id": "c2", "idShort": 2, "name": "Write docs", "desc": "", "due": "2026-11-05T09:00:00.000Z",
     "dateLastActivity": "2026-11-02T10:00:00.000Z", "idList": "L1", "idMembers": [ME], "members": [],
     "labels": [], "closed": False, "customFieldItems": [], "checklists": []},
    {"id": "c3", "idShort": 3, "name": "Ship", "desc": "", "due": "2026-11-12T09:00:00.000Z",
     "dateLastActivity": "2026-11-03T10:00:00.000Z", "idList": "L4", "idMembers": [ME], "members": [],
     "labels": [], "closed": False, "customFieldItems": [], "checklists": []},
    {"id": "c4", "idShort": 4, "name": "Not mine", "desc": "", "due": "2026-11-06T09:00:00.000Z",
     "dateLastActivity": "2026-11-03T10:00:00.000Z", "idList": "L1", "idMembers": [OTHER], "members": [],
     "labels": [], "closed": False, "customFieldItems": [], "checklists": []},
    {"id": "c5", "idShort": 5, "name": "Archived", "desc": "", "due": "2026-11-07T09:00:00.000Z",
     "dateLastActivity": "2026-11-03T10:00:00.000Z", "idList": "L9", "idMembers": [ME], "members": [],
     "labels": [], "closed": True, "customFieldItems": [], "checklists": []},
    {"id": "c6", "idShort": 6, "name": "December", "desc": "", "due": "2026-12-02T09:00:00.000Z",
     "dateLastActivity": "2026-11-03T10:00:00.000Z", "idList": "L1", "idMembers": [ME], "members": [],
     "labels": [], "closed": False, "customFieldItems": [], "checklists": []},
    {"id": "c7", "idShort": 7, "name": "Multi", "desc": "", "due": None,
     "dateLastActivity": "2026-11-03T10:00:00.000Z", "idList": "L5", "idMembers": [ME], "members": [],
     "labels": [], "closed": False, "customFieldItems": [],
     "checklists": [{"name": "B", "pos": 2, "checkItems": [{"name": "b1", "state": "complete", "pos": 1}]},
                    {"name": "A", "pos": 1, "checkItems": [{"name": "a1", "state": "incomplete", "pos": 1}]},
                    {"name": "Empty", "pos": 3, "checkItems": []}]},
]


class FakeTrello:
    """Answers the endpoints the plugin uses. `calls` records (method, path, query) of every request."""

    def __init__(self, auth=AUTH):
        self.auth = auth
        self.calls = []
        self.urls = []
        self.lists = [dict(l) for l in LISTS]
        self.cards = [dict(c) for c in CARDS]
        self.fail = None  # (status, body) to answer every call with
        self.raise_exc = None

    def __call__(self, method, url, headers, body, timeout):
        self.urls.append(url)
        parsed = urllib.parse.urlparse(url)
        assert parsed.scheme == "https" and parsed.netloc == "api.trello.com", url
        path = [urllib.parse.unquote(p) for p in parsed.path.split("/")[2:]]
        query = dict(urllib.parse.parse_qsl(parsed.query))
        self.calls.append((method, "/".join(path), query))
        if self.raise_exc:
            raise self.raise_exc
        if self.fail:
            return self.fail[0], self.fail[1].encode()
        if headers.get("Authorization") != self.auth:
            return 401, b"invalid token"
        return self.route(method, path, query)

    def ok(self, data):
        return 200, json.dumps(data).encode()

    def route(self, method, path, query):
        if path[0] == "boards":
            if path[1] not in (BOARD_ID, SHORT):
                return (400, b"invalid id") if len(path[1]) not in (8, 24) else (404, b"The requested resource was not found.")
            if len(path) == 2:
                return self.ok({"id": BOARD_ID, "shortLink": SHORT, "name": "Team", "idOrganization": "org1"})
            if path[2] == "lists":
                which = query.get("filter", "open")
                return self.ok([l for l in self.lists if which == "all" or not l["closed"]])
            if path[2] == "customFields":
                return self.ok(CUSTOM_FIELDS)
            if path[2] == "cards":
                which = path[3] if len(path) > 3 else "visible"
                cards = [c for c in self.cards if which == "all" or not c["closed"]]
                out = []
                for c in cards:
                    c = dict(c)
                    if query.get("checklists") != "all":
                        c.pop("checklists", None)
                    if query.get("customFieldItems") != "true":
                        c.pop("customFieldItems", None)
                    if query.get("members") != "true":
                        c.pop("members", None)
                    out.append(c)
                return self.ok(out)
        if path[0] == "organizations":
            if path[1] == "my-team":
                return self.ok({"id": "org1", "name": "my-team"})
            if path[1] == "other":
                return self.ok({"id": "org2", "name": "other"})
            return 404, b"model not found"
        if path == ["members", "me"]:
            return self.ok({"id": ME})
        if method == "PUT" and path[0] == "cards":
            card = next((c for c in self.cards if c["id"] == path[1]), None)
            if card is None:
                return 404, b"card not found"
            card["idList"] = query["idList"]
            card["dateLastActivity"] = "2026-11-26T12:00:00.000Z"
            return self.ok(card)
        return 404, b"no route"
