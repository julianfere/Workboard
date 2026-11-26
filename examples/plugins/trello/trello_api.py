"""Trello logic of the Workboard tracker plugin (Trello REST API v1). Python 3.8+, stdlib only.

It knows nothing about the plugin protocol: `plugin.py` wires it to the SDK. Everything that talks to
Trello goes through an injectable `http(method, url, headers, body, timeout) -> (status, bytes)` so the
module is testable without network.

Credentials: the secret is "API key:token". They travel in the `Authorization: OAuth ...` header (never
in the query string), and every error text is passed through `redact` anyway.
"""
import json
import re
import unicodedata
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.trello.com/1"
TIMEOUT = 30

HEX_ID = re.compile(r"^[0-9a-fA-F]{24}$")
SHORT_LINK = re.compile(r"^[0-9A-Za-z]{8}$")
BOARD_URL = re.compile(r"^(?:https?://)?(?:www\.)?trello\.com/b/([0-9A-Za-z]+)(?:[/?#].*)?$")
WORKSPACE_URL = re.compile(r"^(?:https?://)?(?:www\.)?trello\.com/(?:w/)?([0-9A-Za-z_-]+)(?:[/?#].*)?$")
WORKSPACE_NAME = re.compile(r"^[0-9A-Za-z_-]+$")
CARD_NUMBER = re.compile(r"^[0-9]+$")

CARD_FIELDS = "id,idShort,name,desc,due,dateLastActivity,idList,idMembers,labels,closed"

# Trello's named colors (lists and labels) as hex without '#'.
COLORS = {
    "green": "61BD4F", "yellow": "F2D600", "orange": "FF9F1A", "red": "EB5A46", "purple": "C377E0",
    "blue": "0079BF", "sky": "00C2E0", "lime": "51E898", "pink": "FF78CB", "black": "344563",
}

# Guess of the workflow category from a list name (accents and case ignored). First match wins.
SUGGESTED = [
    ("cancelled", r"cancel+ed|cancelad[oa]s?|won'?t do|descartad[oa]s?"),
    ("done", r"done|hecho|terminad[oa]s?|completed?|completad[oa]s?|finished|finalizad[oa]s?|listo|cerrad[oa]s?"),
    ("review", r"review|revision|en revision|qa|testing|test|pruebas?|validacion"),
    ("active", r"doing|in progress|en progreso|en curso|haciendo|wip|working"),
    ("backlog", r"backlog|to ?do|por hacer|pendientes?|ideas"),
]


class TrelloError(Exception):
    """An error for the user. `code` is `tracker.auth`, `tracker.notFound` or `tracker.error`."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code
        self.message = message


# ---------------------------------------------------------------------------------------- secrets

def parse_secret(secret):
    """'API key:token' -> (key, token). Whitespace around each part is ignored."""
    key, sep, token = (secret or "").partition(":")
    key, token = key.strip(), token.strip()
    if not sep or not key or not token:
        raise TrelloError("tracker.auth", "The credential must be 'API key:token'.")
    return key, token


def redact(text, secret):
    """Removes the credential (whole, key and token) from a text."""
    text = str(text)
    if not secret:
        return text
    parts = [secret, secret.strip()]
    key, _, token = secret.partition(":")
    parts += [key.strip(), token.strip()]
    for part in sorted({p for p in parts if len(p) >= 4}, key=len, reverse=True):
        text = text.replace(part, "***")
        text = text.replace(urllib.parse.quote(part, safe=""), "***")
    return text


# ------------------------------------------------------------------------------------------- http

def urllib_http(method, url, headers, body, timeout):
    """Default transport. Returns (status, body bytes); HTTP errors are returned, not raised."""
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as e:
        try:
            data = e.read()
        except Exception:  # pragma: no cover - body unavailable
            data = b""
        return e.code, data


class Client:
    """Authenticated calls to the Trello API."""

    def __init__(self, secret, http=None, timeout=TIMEOUT):
        self.secret = secret or ""
        self.key, self.token = parse_secret(self.secret)
        self.http = http or urllib_http
        self.timeout = timeout

    def url(self, path, params=None):
        segments = "/".join(urllib.parse.quote(str(s), safe="") for s in path)
        query = ("?" + urllib.parse.urlencode(params)) if params else ""
        return "%s/%s%s" % (API, segments, query)

    def call(self, method, path, params=None, what=None):
        """`path` is a list of segments (each one is URL-encoded). `what` names the resource for 404s."""
        headers = {
            "Accept": "application/json",
            "Authorization": 'OAuth oauth_consumer_key="%s", oauth_token="%s"' % (self.key, self.token),
        }
        try:
            status, data = self.http(method, self.url(path, params), headers, None, self.timeout)
        except TrelloError:
            raise
        except Exception as e:  # network, TLS, timeout...
            raise TrelloError("tracker.error", redact("Could not reach Trello: %s" % (e,), self.secret))
        text = (data or b"").decode("utf-8", "replace")
        if 200 <= status < 300:
            try:
                return json.loads(text) if text.strip() else None
            except ValueError:
                raise TrelloError("tracker.error", "Trello answered something that is not JSON.")
        detail = redact(_error_text(text), self.secret)
        low = detail.lower()
        if status == 401 or "invalid key" in low or "invalid token" in low or "unauthorized" in low:
            raise TrelloError("tracker.auth", "Trello rejected the API key or token (%s)." % (detail or status))
        if status == 404 or (status == 400 and "invalid id" in low):
            raise TrelloError("tracker.notFound", "%s not found in Trello." % (what or "Resource"))
        if status == 429:
            raise TrelloError("tracker.error", "Trello rate limit reached; try again in a few seconds.")
        raise TrelloError("tracker.error", "Trello error %s: %s" % (status, detail or "no details"))

    def get(self, path, params=None, what=None):
        return self.call("GET", path, params, what)

    def put(self, path, params=None, what=None):
        return self.call("PUT", path, params, what)


def _error_text(text):
    text = text.strip()
    try:
        obj = json.loads(text)
        if isinstance(obj, dict):
            text = str(obj.get("message") or obj.get("error") or text)
    except ValueError:
        pass
    return text[:300]


# -------------------------------------------------------------------------------------- normalize

def normalize_board(project):
    """Board id, shortLink or URL -> the id/shortLink (as given). Raises tracker.notFound if malformed."""
    value = (project or "").strip()
    m = BOARD_URL.match(value)
    if m:
        value = m.group(1)
    if HEX_ID.match(value) or SHORT_LINK.match(value):
        return value
    raise TrelloError("tracker.notFound", "Not a Trello board id, short link or URL.")


def normalize_workspace(org):
    """Optional workspace name or URL -> its name ('' when empty)."""
    value = (org or "").strip()
    if not value:
        return ""
    m = WORKSPACE_URL.match(value)
    if m and "trello.com" in value:
        value = m.group(1)
    if WORKSPACE_NAME.match(value):
        return value
    raise TrelloError("tracker.notFound", "Not a Trello workspace name or URL.")


def suggest(name):
    """Workflow category guessed from a list name, or None."""
    low = unicodedata.normalize("NFKD", name or "").encode("ascii", "ignore").decode().lower().strip()
    for category, pattern in SUGGESTED:
        if re.search(r"(?<![a-z])(?:%s)(?![a-z])" % pattern, low):
            return category
    return None


def checklist_markdown(checklists):
    """Card checklists -> a Markdown checklist (a bold title per checklist when there is more than one)."""
    out = []
    lists = sorted(checklists or [], key=lambda c: c.get("pos") or 0)
    for cl in lists:
        items = sorted(cl.get("checkItems") or [], key=lambda i: i.get("pos") or 0)
        if not items:
            continue
        if len(lists) > 1:
            if out:
                out.append("")
            out.append("**%s**" % (cl.get("name") or "").strip())
        for it in items:
            mark = "x" if it.get("state") == "complete" else " "
            out.append("- [%s] %s" % (mark, (it.get("name") or "").strip()))
    return "\n".join(out)


def _number(value):
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _day(iso):
    return (iso or "")[:10]


# ------------------------------------------------------------------------------------------ board

class Board:
    """One connection: a board (and optionally its workspace) plus the credential."""

    def __init__(self, conn, secret, http=None):
        conn = conn or {}
        self.org = (conn.get("org") or "").strip()
        self.project = (conn.get("project") or "").strip()
        self.client = Client(secret, http)

    @property
    def board(self):
        return normalize_board(self.project)

    # ---- reads

    def _lists(self, which="all"):
        return self.client.get(["boards", self.board, "lists"],
                               {"filter": which, "fields": "id,name,closed,color,pos"}, what="Board") or []

    def _custom_fields(self):
        return self.client.get(["boards", self.board, "customFields"], what="Board") or []

    def _cards(self, which, text=False):
        params = {"fields": CARD_FIELDS, "members": "true", "member_fields": "fullName",
                  "customFieldItems": "true"}
        if text:
            params["checklists"] = "all"
            params["checklist_fields"] = "name,pos"
        return self.client.get(["boards", self.board, "cards", which], params, what="Board") or []

    def _cards_by_number(self, cards):
        out = {}
        for c in cards:
            n = c.get("idShort")
            if n is not None and str(n) not in out:
                out[str(n)] = c
        return out

    def _card(self, id):
        if not CARD_NUMBER.match(str(id or "")):
            raise TrelloError("tracker.notFound", "Card #%s not found in Trello." % id)
        return id

    # ---- methods

    def connect(self):
        board = self.client.get(["boards", self.board], {"fields": "id,shortLink,name,idOrganization"},
                                what="Board")
        org = normalize_workspace(self.org)
        if org:
            ws = self.client.get(["organizations", org], {"fields": "id,name"}, what="Workspace")
            if ws.get("id") != board.get("idOrganization"):
                raise TrelloError("tracker.notFound", "The board is not in that workspace.")
            org = ws.get("name") or org
        return {"org": org, "project": board.get("shortLink") or board.get("id") or self.board}

    def states(self):
        out = []
        for lst in sorted(self._lists("open"), key=lambda l: l.get("pos") or 0):
            if lst.get("closed"):
                continue
            name = lst.get("name") or ""
            out.append({"name": name, "color": COLORS.get(lst.get("color") or "", ""), "category": "list",
                        "types": [], "suggested": suggest(name)})
        return out

    def fields(self):
        out = [{"ref": "due", "name": "Due date", "type": "date", "role": "due"}]
        for cf in self._custom_fields():
            kind = cf.get("type") or ""
            field = {"ref": cf.get("id"), "name": cf.get("name") or "", "type": kind}
            if kind == "number" and re.search(r"hour|hora|estimat", (cf.get("name") or "").lower()):
                field["role"] = "hours"
            out.append(field)
        return out

    def work_items(self, ids, request):
        request = request or {}
        wanted = [str(i) for i in (ids or []) if CARD_NUMBER.match(str(i))]
        if not wanted:
            return {}
        text = bool(request.get("text"))
        cards = self._cards_by_number(self._cards("all", text))
        lists = {l.get("id"): l.get("name") or "" for l in self._lists("all")}
        refs = {request.get("hours"), request.get("due")} | set(request.get("extra") or [])
        refs.discard(None)
        refs.discard("due")
        custom = {cf.get("id"): cf for cf in self._custom_fields()} if refs else {}
        out = {}
        for id in wanted:
            card = cards.get(id)
            if card is None:
                continue
            values = self._custom_values(card, custom)
            due_ref = request.get("due")
            due = (card.get("due") or "") if due_ref in (None, "due") else (values.get(due_ref) or "")
            hours = request.get("hours")
            item = {
                "state": lists.get(card.get("idList"), ""),
                "title": card.get("name") or "",
                "type": _first_label(card),
                "assigned": ", ".join(m.get("fullName") or "" for m in card.get("members") or []),
                "changed": card.get("dateLastActivity") or "",
                "priority": None,
                "due": due if isinstance(due, str) else "",
                "estimate": _number(values.get(hours)) if hours else None,
                "completed": None,
                "fields": {},
            }
            for ref in request.get("extra") or []:
                item["fields"][ref] = card.get("due") if ref == "due" else values.get(ref)
            if text:
                item["description"] = card.get("desc") or ""
                item["criteria"] = checklist_markdown(card.get("checklists"))
            out[id] = item
        return out

    def _custom_values(self, card, custom):
        """{customFieldId: value} of a card (number -> float, date/text -> str, checkbox -> bool, list -> text)."""
        out = {}
        for it in card.get("customFieldItems") or []:
            ref = it.get("idCustomField")
            value = it.get("value") or {}
            if "number" in value:
                out[ref] = _number(value["number"])
            elif "date" in value:
                out[ref] = value["date"]
            elif "text" in value:
                out[ref] = value["text"]
            elif "checked" in value:
                out[ref] = str(value["checked"]).lower() == "true"
            elif it.get("idValue"):
                options = (custom.get(ref) or {}).get("options") or []
                out[ref] = next(((o.get("value") or {}).get("text") for o in options
                                 if o.get("id") == it.get("idValue")), None)
        return out

    def assigned_to_me(self, due_field, start, end, exclude):
        me = self.client.get(["members", "me"], {"fields": "id"}) or {}
        lists = {l.get("id"): l.get("name") or "" for l in self._lists("all")}
        skip = {e for e in exclude or []}
        field = due_field or "due"
        found = []
        for card in self._cards("open"):
            if me.get("id") not in (card.get("idMembers") or []):
                continue
            if lists.get(card.get("idList"), "") in skip:
                continue
            if field == "due":
                due = card.get("due") or ""
            else:
                due = self._custom_values(card, {}).get(field) or ""
            if not isinstance(due, str) or not due:
                continue
            day = _day(due)
            if (start and day < _day(start)) or (end and day >= _day(end)):
                continue
            found.append((due, card.get("idShort") or 0))
        found.sort()
        return [str(n) for _, n in found]

    def detail(self, id):
        self._card(id)
        card = self._cards_by_number(self._cards("all", text=True)).get(str(id))
        if card is None:
            raise TrelloError("tracker.notFound", "Card #%s not found in Trello." % id)
        lists = {l.get("id"): l.get("name") or "" for l in self._lists("all")}
        return {
            "id": str(id),
            "title": card.get("name") or "",
            "type": _first_label(card),
            "state": lists.get(card.get("idList"), ""),
            "description": card.get("desc") or "",
            "criteria": checklist_markdown(card.get("checklists")),
        }

    def set_state(self, id, state):
        """WRITES: moves the card to the open list named `state`."""
        self._card(id)
        target = next((l for l in sorted(self._lists("open"), key=lambda l: l.get("pos") or 0)
                       if (l.get("name") or "") == state and not l.get("closed")), None)
        if target is None:
            raise TrelloError("tracker.error", "The board has no open list named '%s'." % state)
        cards = self.client.get(["boards", self.board, "cards", "all"], {"fields": "id,idShort"},
                                what="Board") or []
        card = self._cards_by_number(cards).get(str(id))
        if card is None:
            raise TrelloError("tracker.notFound", "Card #%s not found in Trello." % id)
        moved = self.client.put(["cards", card["id"]], {"idList": target["id"]}, what="Card") or {}
        return {"state": target.get("name") or state, "changed": moved.get("dateLastActivity") or ""}


def _first_label(card):
    for label in card.get("labels") or []:
        if label.get("name"):
            return label["name"]
    return ""
