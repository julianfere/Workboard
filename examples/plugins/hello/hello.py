"""Minimal Workboard plugin (see docs/plugins.md). Python 3.8+, no dependencies.

One process per step: read `initialize`, answer it, read the call, answer it and exit.
"""
import json
import sys

TEXTS = {
    "en": {"title": "Hello", "name": "Your name", "greet": "Greet", "session": "Open a session",
           "done": "Hello, {name}!", "items": "Started on: {items}", "none": "(no items)"},
    "es": {"title": "Hola", "name": "Tu nombre", "greet": "Saludar", "session": "Abrir una sesión",
           "done": "¡Hola, {name}!", "items": "Empezó en: {items}", "none": "(ningún trabajo)"},
}


def send(msg):
    sys.stdout.write(json.dumps(msg, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def request(next_id, method, params):
    """Asks Workboard something and waits for the answer (not used by this example's flow)."""
    send({"jsonrpc": "2.0", "id": next_id, "method": method, "params": params})
    for line in sys.stdin:
        msg = json.loads(line)
        if msg.get("id") == next_id:
            if "error" in msg:
                raise RuntimeError(msg["error"]["message"])
            return msg["result"]
    raise RuntimeError("Workboard closed the connection")


def handle(method, params, lang):
    tx = TEXTS.get(lang, TEXTS["en"])
    items = [it["name"] for it in params["context"]["items"]]
    if method == "action/open":
        return {
            "type": "form",
            "title": tx["title"],
            "intro": tx["items"].format(items=", ".join(items) or tx["none"]),
            "fields": [{"id": "name", "type": "text", "label": tx["name"], "required": True}],
            "buttons": [{"id": "greet", "label": tx["greet"], "primary": True}]
            + ([{"id": "session", "label": tx["session"]}] if items else []),
        }
    name = params["values"].get("name", "")
    if params["button"] == "session":
        return {"type": "session", "items": items, "name": tx["title"], "mode": "plan",
                "prompt": tx["done"].format(name=name)}
    return {"type": "result", "title": tx["title"], "ok": True, "message": tx["done"].format(name=name),
            "steps": [{"label": tx["items"].format(items=", ".join(items) or tx["none"]), "ok": True}]}


def main():
    lang = "en"
    for line in sys.stdin:
        msg = json.loads(line)
        method, msg_id = msg.get("method"), msg.get("id")
        if method == "initialize":
            lang = msg["params"].get("lang", "en")
            send({"jsonrpc": "2.0", "id": msg_id, "result": {}})
            continue
        try:
            send({"jsonrpc": "2.0", "id": msg_id, "result": handle(method, msg["params"], lang)})
        except Exception as exc:  # noqa: BLE001 - reported to Workboard
            send({"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32000, "message": str(exc)}})


if __name__ == "__main__":
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    main()
