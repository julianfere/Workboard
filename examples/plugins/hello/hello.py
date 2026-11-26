"""Minimal Workboard plugin (see docs/plugins.md), written with the Python SDK.

`workboard_plugin.py` is a copy of `sdk/python/workboard_plugin.py`, so this folder works on its own once
copied into `<config>/plugins/`. One process per step: the SDK answers `initialize`, calls the handler for
the step and exits when Workboard closes stdin.
"""
from workboard_plugin import Plugin, button, form, result, session, text

TEXTS = {
    "en": {"title": "Hello", "name": "Your name", "greet": "Greet", "session": "Open a session",
           "done": "Hello, {name}!", "items": "Started on: {items}", "none": "(no items)"},
    "es": {"title": "Hola", "name": "Tu nombre", "greet": "Saludar", "session": "Abrir una sesión",
           "done": "¡Hola, {name}!", "items": "Empezó en: {items}", "none": "(ningún trabajo)"},
}

plugin = Plugin()


def started_on(ctx, tx):
    return tx["items"].format(items=", ".join(it["name"] for it in ctx.items) or tx["none"])


@plugin.action("greet")
def greet(ctx):
    tx = TEXTS.get(ctx.lang, TEXTS["en"])
    buttons = [button("greet", tx["greet"], primary=True)]
    if ctx.items:
        buttons.append(button("session", tx["session"]))
    return form(tx["title"], [text("name", tx["name"], required=True)], buttons=buttons,
                intro=started_on(ctx, tx))


@greet.submit
def greet_submit(ctx, button, values, state):
    tx = TEXTS.get(ctx.lang, TEXTS["en"])
    name = values.get("name", "")
    if button == "session":
        return session([it["name"] for it in ctx.items], prompt=tx["done"].format(name=name),
                       name=tx["title"], mode="plan")
    return result(tx["title"], tx["done"].format(name=name), steps=[(started_on(ctx, tx), True)])


if __name__ == "__main__":
    plugin.run()
