"""Workboard plugin SDK for Python (3.8+, standard library only, single file).

Copy this file next to your plugin and import it. It speaks Workboard's plugin protocol (JSON-RPC 2.0,
one object per line on stdin/stdout, see ``docs/plugins.md``): it answers ``initialize``, routes
``action/open``, ``action/submit`` and ``tracker/*`` calls to your handlers, lets them call Workboard
(``ctx.host``) while they run and turns exceptions into JSON-RPC errors.

Example::

    from workboard_plugin import Plugin, form, text, button, result

    plugin = Plugin()

    @plugin.action("greet")
    def greet(ctx):
        return form("Hello", [text("name", "Your name", required=True)],
                    buttons=[button("greet", "Greet", primary=True)])

    @greet.submit
    def greet_submit(ctx, button, values, state):
        return result("Hello", "Hello, %s!" % values["name"])

    plugin.run()

Anything a handler prints goes to stderr (Workboard's log), never to the protocol stream.
"""

import json
import sys
import traceback

__all__ = [
    "PROTOCOL", "Plugin", "Context", "Connection", "Host", "HostError", "PluginError", "TrackerError",
    "form", "text", "textarea", "password", "select", "checkbox", "number", "info", "button",
    "result", "session", "close",
]

__version__ = "1.0.0"

#: Newest protocol version this SDK speaks.
PROTOCOL = 1

#: JSON-RPC error codes used in answers.
ERROR_FAILED = -32000
ERROR_METHOD = -32601

_TRACKER_METHODS = {
    "tracker/connect": "connect",
    "tracker/states": "states",
    "tracker/fields": "fields",
    "tracker/workItems": "work_items",
    "tracker/assignedToMe": "assigned_to_me",
    "tracker/detail": "detail",
    "tracker/setState": "set_state",
}


# --------------------------------------------------------------------------------------------- errors


class PluginError(Exception):
    """An error whose message is shown to the user as is (no traceback is logged)."""


class TrackerError(PluginError):
    """A tracker error. Its JSON-RPC ``message`` is ``code`` when given, so Workboard can tell
    ``tracker.auth`` (rejected credential) and ``tracker.notFound`` (missing project) apart;
    ``message`` then travels as the error's ``data.detail``."""

    def __init__(self, code=None, message=None):
        self.code = code
        self.message = message or code or "tracker error"
        super().__init__(code or self.message)


class HostError(Exception):
    """Workboard answered a ``host/...`` request with an error. ``code`` is its message, a code such
    as ``plugin.denied``, ``plugin.badArgument`` or ``core.…``."""

    def __init__(self, method, code):
        self.method = method
        self.code = code
        super().__init__("%s failed: %s" % (method, code))


# ------------------------------------------------------------------------------------------- builders


def _clean(d):
    """Drops keys whose value is ``None``."""
    return {k: v for k, v in d.items() if v is not None}


def _field(kind, id, label, value=None, required=False, wide=False, hint=None, placeholder=None,
           refresh=False, **extra):
    f = {"id": id, "type": kind, "label": label, "value": value, "hint": hint, "placeholder": placeholder}
    if required:
        f["required"] = True
    if wide:
        f["wide"] = True
    if refresh:
        f["refresh"] = True
    f.update(extra)
    return _clean(f)


def text(id, label, value=None, required=False, wide=False, hint=None, placeholder=None, refresh=False):
    """A single-line text field."""
    return _field("text", id, label, value, required, wide, hint, placeholder, refresh)


def textarea(id, label, value=None, rows=None, required=False, wide=False, hint=None, placeholder=None,
             refresh=False):
    """A multi-line text field; ``rows`` is its height."""
    return _field("textarea", id, label, value, required, wide, hint, placeholder, refresh, rows=rows)


def password(id, label, required=False, wide=False, hint=None, placeholder=None):
    """A password field. Its value only comes back in ``values`` when the user typed something."""
    return _field("password", id, label, None, required, wide, hint, placeholder)


def _option(o):
    if isinstance(o, dict):
        return o
    if isinstance(o, (tuple, list)):
        return {"value": o[0], "label": o[1] if len(o) > 1 else o[0]}
    return {"value": o, "label": str(o)}


def select(id, label, options, value=None, required=False, wide=False, hint=None, refresh=False):
    """A drop-down. ``options`` are ``{"value", "label"}`` dicts, ``(value, label)`` tuples or plain
    values (shown as they are)."""
    return _field("select", id, label, value, required, wide, hint, None, refresh,
                  options=[_option(o) for o in options])


def checkbox(id, label, value=False, wide=False, hint=None, refresh=False):
    """A checkbox; its value comes back as a boolean."""
    return _field("checkbox", id, label, bool(value), False, wide, hint, None, refresh)


def number(id, label, value=None, required=False, wide=False, hint=None, placeholder=None, refresh=False):
    """A number field; its value comes back as a number."""
    return _field("number", id, label, value, required, wide, hint, placeholder, refresh)


def info(text, id=None, wide=False):
    """A paragraph of plain text inside the form."""
    f = {"type": "info", "text": text, "id": id}
    if wide:
        f["wide"] = True
    return _clean(f)


def button(id, label, primary=False, validate=None, confirm=None):
    """A form or result button. ``validate=False`` submits without checking required fields;
    ``confirm`` asks the user before submitting."""
    b = {"id": id, "label": label, "confirm": confirm}
    if primary:
        b["primary"] = True
    if validate is not None:
        b["validate"] = bool(validate)
    return _clean(b)


def _button(b):
    if isinstance(b, dict):
        return b
    return button(*b)


def form(title, fields, buttons=None, intro=None, error=None, state=None):
    """A form step. ``buttons`` are :func:`button` dicts or ``(id, label)`` tuples (Workboard always
    adds Cancel); ``state`` comes back untouched in the submit handler."""
    return _clean({
        "type": "form", "title": title, "intro": intro, "error": error, "fields": list(fields),
        "buttons": [_button(b) for b in (buttons or [])], "state": state,
    })


def _step(s):
    if isinstance(s, dict):
        return s
    label, ok = s[0], (s[1] if len(s) > 1 else True)
    return _clean({"label": label, "ok": bool(ok), "error": s[2] if len(s) > 2 else None})


def _link(l):
    if isinstance(l, dict):
        return l
    return {"label": l[0], "url": l[1]}


def result(title, message=None, ok=True, steps=None, links=None, buttons=None, state=None):
    """A result step. ``steps`` are dicts or ``(label, ok[, error])`` tuples, ``links`` dicts or
    ``(label, url)`` tuples (only ``https://`` links are shown); ``buttons`` submit again with ``state``."""
    r = {"type": "result", "title": title, "ok": bool(ok), "message": message, "state": state}
    if steps:
        r["steps"] = [_step(s) for s in steps]
    if links:
        r["links"] = [_link(l) for l in links]
    if buttons:
        r["buttons"] = [_button(b) for b in buttons]
    return _clean(r)


def session(items, prompt=None, name=None, mode="default", model=None):
    """Opens an agent session on ``items`` (item names). ``mode`` is ``default``, ``plan`` or
    ``acceptEdits``."""
    return _clean({"type": "session", "items": list(items), "name": name, "mode": mode, "prompt": prompt,
                   "model": model})


def close(message=None):
    """Closes the dialog and shows ``message``."""
    return _clean({"type": "close", "message": message})


# -------------------------------------------------------------------------------------- host and ctx


class Host:
    """Requests to Workboard while a call is handled. Each method raises :class:`HostError` when
    Workboard answers an error (for example ``plugin.denied`` without the permission)."""

    def __init__(self, rpc):
        self._rpc = rpc

    def request(self, method, params=None):
        """Sends any ``host/...`` request and returns its result."""
        return self._rpc.request(method, params or {})

    def workflow(self):
        """The active workflow (preset JSON)."""
        return self.request("host/workflow")

    def items(self):
        """The items of the open folder (``ItemRef`` dicts)."""
        return self.request("host/items")

    def item(self, name):
        """One item with its ``meta`` (frontmatter) and ``files``."""
        return self.request("host/item", {"name": name})

    def update_item(self, name, fields):
        """Writes frontmatter fields (``""`` deletes one). Needs ``items.write``."""
        return self.request("host/updateItem", {"name": name, "fields": fields})

    def read_file(self, path):
        """The text of a file inside an item folder (path relative to the root). Needs ``files.read``."""
        return self.request("host/readFile", {"path": path}).get("text")

    def secret_get(self, key):
        """A secret of this plugin, or ``None``. Needs ``secrets``."""
        return self.request("host/secret/get", {"key": key}).get("value")

    def secret_set(self, key, value):
        """Stores a secret in the system credential store. Needs ``secrets``."""
        self.request("host/secret/set", {"key": key, "value": value})

    def secret_delete(self, key):
        """Removes a secret. Needs ``secrets``."""
        self.request("host/secret/delete", {"key": key})

    def data_get(self):
        """This plugin's JSON data (``{}`` if none)."""
        return self.request("host/data/get")

    def data_set(self, data):
        """Replaces this plugin's JSON data (at most 1 MB)."""
        self.request("host/data/set", {"data": data})

    def tracker(self):
        """The open folder's tracker connection (``kind``, ``source``, ``org``, ``project``) or ``None``.
        Needs ``tracker``."""
        return self.request("host/tracker")

    def tracker_write_token_get(self):
        """The shared tracker write token, or ``None``. Needs ``tracker.write``."""
        return self.request("host/tracker/writeToken/get").get("value")

    def tracker_write_token_set(self, value):
        """Replaces the shared tracker write token. Needs ``tracker.write``."""
        self.request("host/tracker/writeToken/set", {"value": value})

    def tracker_write_token_delete(self):
        """Removes the shared tracker write token. Needs ``tracker.write``."""
        self.request("host/tracker/writeToken/delete")


class Context:
    """Where an action runs: ``lang``, ``root``, ``placement`` (``item``, ``items``, ``palette`` or
    ``settings``), ``items`` (``ItemRef`` dicts), ``plugin_dir``, ``plugin_id``, ``action`` and
    ``host``."""

    def __init__(self, plugin, action, context):
        context = context or {}
        self.action = action
        self.lang = context.get("lang") or plugin.lang
        self.root = context.get("root")
        self.placement = context.get("placement")
        self.items = list(context.get("items") or [])
        self.raw = context
        self.plugin_dir = plugin.plugin_dir
        self.plugin_id = plugin.plugin_id
        self.host = plugin.host

    @property
    def item(self):
        """The first item, or ``None``."""
        return self.items[0] if self.items else None


class Connection(dict):
    """A tracker connection: a dict (``org``, ``project``) whose keys are also attributes."""

    def __getattr__(self, name):
        try:
            return self[name]
        except KeyError:
            raise AttributeError(name)


class Action:
    """An action registered with :meth:`Plugin.action`. Use ``@handler.submit`` to add its submit
    handler; calling the object calls the open handler."""

    def __init__(self, id, open):
        self.id = id
        self.open = open
        self.on_submit = None

    def submit(self, fn):
        """Decorator: ``fn(ctx, button, values, state)`` answers ``action/submit``."""
        self.on_submit = fn
        return fn

    def __call__(self, *args, **kwargs):
        return self.open(*args, **kwargs)


# ---------------------------------------------------------------------------------------------- rpc


class _Rpc:
    def __init__(self, stdin, stdout):
        self._in = stdin
        self._out = stdout
        self._next = 1000
        self.pending = []
        self.closed = False

    def send(self, msg):
        line = json.dumps(msg, ensure_ascii=False, separators=(",", ":")) + "\n"
        self._out.write(line.encode("utf-8"))
        self._out.flush()

    def read(self):
        """The next message, or ``None`` when stdin is closed. Lines that are not JSON are skipped."""
        if self.pending:
            return self.pending.pop(0)
        while True:
            raw = self._in.readline()
            if not raw:
                self.closed = True
                return None
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except ValueError:
                sys.stderr.write("workboard_plugin: ignoring a line that is not JSON\n")
                continue
            if isinstance(msg, dict):
                return msg

    def request(self, method, params):
        self._next += 1
        id = self._next
        self.send({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        while True:
            raw = self._in.readline() if not self.closed else b""
            if not raw:
                self.closed = True
                raise HostError(method, "Workboard closed the connection")
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if not isinstance(msg, dict):
                continue
            if "method" in msg:
                # A request from Workboard while we wait: handled after the current one.
                self.pending.append(msg)
                continue
            if msg.get("id") != id:
                continue
            err = msg.get("error")
            if err is not None:
                code = err.get("message") if isinstance(err, dict) else str(err)
                raise HostError(method, code or "error")
            return msg.get("result")


# ------------------------------------------------------------------------------------------- plugin


class Plugin:
    """A Workboard plugin: register handlers with :meth:`action` and :meth:`tracker`, then call
    :meth:`run`. After ``initialize``, ``lang``, ``plugin_id``, ``plugin_dir`` and ``host`` are set."""

    def __init__(self):
        self.actions = {}
        self.trackers = {}
        self.lang = "en"
        self.plugin_id = None
        self.plugin_dir = None
        self.init = {}
        self.host = None
        self._rpc = None

    def action(self, id):
        """Decorator for the open handler of action ``id``: ``fn(ctx)`` returns a step."""

        def deco(fn):
            a = Action(id, fn)
            self.actions[id] = a
            return a

        return deco

    def tracker(self, kind):
        """Class decorator: the class (instantiated once, without arguments) or an instance answers the
        ``tracker/*`` calls for ``kind`` with ``connect``, ``states``, ``fields``, ``work_items``,
        ``assigned_to_me``, ``detail`` and ``set_state``."""

        def deco(cls):
            self.trackers[kind] = cls() if isinstance(cls, type) else cls
            return cls

        return deco

    # -- dispatch

    def _initialize(self, params):
        params = params or {}
        version = params.get("protocol", PROTOCOL)
        if isinstance(version, int) and version > PROTOCOL:
            raise PluginError("protocol %s is newer than this plugin supports (%s): update the plugin"
                              % (version, PROTOCOL))
        self.init = params
        self.lang = params.get("lang") or "en"
        info_ = params.get("plugin") or {}
        self.plugin_id = info_.get("id")
        self.plugin_dir = info_.get("dir")
        return {}

    def _action(self, method, params):
        action_id = params.get("action")
        a = self.actions.get(action_id)
        if a is None:
            raise PluginError("unknown action %r" % action_id)
        ctx = Context(self, action_id, params.get("context"))
        if method == "action/open":
            step = a.open(ctx)
        else:
            if a.on_submit is None:
                raise PluginError("action %r has no submit handler" % action_id)
            step = a.on_submit(ctx, params.get("button"), params.get("values") or {},
                               params.get("state") or {})
        if not isinstance(step, dict):
            raise PluginError("action %r returned no step" % action_id)
        return step

    def _tracker_impl(self, kind):
        if kind in self.trackers:
            return self.trackers[kind]
        if kind is None and len(self.trackers) == 1:
            return next(iter(self.trackers.values()))
        raise TrackerError(None, "unknown tracker kind %r" % kind)

    def _tracker(self, method, params):
        impl = self._tracker_impl(params.get("kind"))
        name = _TRACKER_METHODS[method]
        fn = getattr(impl, name, None)
        if fn is None:
            raise PluginError("%s is not implemented" % method)
        conn = Connection(params.get("connection") or {})
        secret = params.get("secret")
        if method == "tracker/workItems":
            args = (params.get("ids") or [], params.get("request") or {})
        elif method == "tracker/assignedToMe":
            args = (params.get("dueField"), params.get("start"), params.get("end"),
                    params.get("exclude") or [])
        elif method == "tracker/detail":
            args = (params.get("id"),)
        elif method == "tracker/setState":
            args = (params.get("id"), params.get("state"))
        else:
            args = ()
        out = fn(conn, secret, *args)
        if out is None and method == "tracker/connect":
            # Nothing to normalize: the connection is accepted as it is.
            out = {"org": conn.get("org", ""), "project": conn.get("project", "")}
        return out

    def handle(self, method, params):
        """Answers one call; raises on errors. Mostly for tests: :meth:`run` uses it."""
        params = params if isinstance(params, dict) else {}
        if method == "initialize":
            return self._initialize(params)
        if method in ("action/open", "action/submit"):
            return self._action(method, params)
        if method in _TRACKER_METHODS:
            return self._tracker(method, params)
        raise _MethodNotFound(method)

    def _answer(self, msg):
        method, id = msg.get("method"), msg.get("id")
        secret = (msg.get("params") or {}).get("secret") if isinstance(msg.get("params"), dict) else None
        try:
            out = self.handle(method, msg.get("params"))
            reply = {"jsonrpc": "2.0", "id": id, "result": out}
        except _MethodNotFound:
            reply = {"jsonrpc": "2.0", "id": id,
                     "error": {"code": ERROR_METHOD, "message": "unknown method %s" % method}}
        except TrackerError as e:
            err = {"code": ERROR_FAILED, "message": _redact(e.code or e.message, secret)}
            if e.code and e.message != e.code:
                err["data"] = {"detail": _redact(e.message, secret)}
            reply = {"jsonrpc": "2.0", "id": id, "error": err}
        except PluginError as e:
            reply = {"jsonrpc": "2.0", "id": id,
                     "error": {"code": ERROR_FAILED, "message": _redact(str(e), secret)}}
        except Exception as e:  # noqa: BLE001 - every error goes back to Workboard
            sys.stderr.write(_redact(traceback.format_exc(), secret))
            if isinstance(e, HostError):
                text = str(e)
            elif str(e):
                text = "%s: %s" % (type(e).__name__, e)
            else:
                text = type(e).__name__
            reply = {"jsonrpc": "2.0", "id": id, "error": {"code": ERROR_FAILED, "message": _redact(text, secret)}}
        if id is not None:
            self._rpc.send(reply)

    def run(self, stdin=None, stdout=None):
        """Serves Workboard until it closes stdin. ``stdin``/``stdout`` are binary streams (default:
        the process's). While a handler runs, ``sys.stdout`` points to stderr so a stray ``print``
        cannot break the protocol."""
        _reconfigure()
        stdin = stdin or sys.stdin.buffer
        stdout = stdout or sys.stdout.buffer
        self._rpc = _Rpc(stdin, stdout)
        self.host = Host(self._rpc)
        saved = sys.stdout
        sys.stdout = sys.stderr
        try:
            while True:
                msg = self._rpc.read()
                if msg is None:
                    return
                if "method" in msg:
                    self._answer(msg)
        finally:
            sys.stdout = saved


class _MethodNotFound(Exception):
    pass


def _redact(text, secret):
    if secret and isinstance(secret, str) and len(secret) >= 4:
        return text.replace(secret, "***")
    return text


def _reconfigure():
    """UTF-8 on stderr (stdin and stdout are used as bytes), also on Windows consoles."""
    for stream in (sys.stderr,):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
