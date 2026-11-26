"""Plugin used by the SDK tests: exercises every part of the SDK."""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from workboard_plugin import (  # noqa: E402
    HostError, Plugin, TrackerError, button, close, form, info, result, select, session, text,
)

plugin = Plugin()


@plugin.action("greet")
def greet(ctx):
    print("stray print goes to stderr")
    item = ctx.host.item(ctx.items[0]["name"]) if ctx.items else {"title": "-"}
    try:
        ctx.host.secret_get("token")
        denied = None
    except HostError as e:
        denied = e.code
    return form(
        "%s · %s" % (item["title"], ctx.lang),
        [text("name", "Name", required=True), select("flow", "Flow", [("a", "A"), "b"], refresh=True),
         info("Some help")],
        buttons=[("more", "More"), button("greet", "Greet", primary=True, confirm="Sure?")],
        state={"denied": denied, "placement": ctx.placement, "dir": ctx.plugin_dir},
    )


@greet.submit
def greet_submit(ctx, button, values, state):
    if button == "session":
        return session([i["name"] for i in ctx.items], prompt="Hi", mode="plan")
    if button == "close":
        return close("Saved")
    ctx.host.update_item(ctx.items[0]["name"], {"notes": "greeted %s" % values["name"]})
    return result("Hello", "¡Hola, %s!" % values["name"], steps=[("Saved", True)],
                  links=[("Open", "https://example.com")], state=state)


@plugin.action("boom")
def boom(ctx):
    raise ValueError("boom")


@plugin.action("hosterr")
def hosterr(ctx):
    ctx.host.read_file("x.txt")


@plugin.tracker("fake")
class Fake:
    def connect(self, conn, secret):
        if secret != "s3cret-key":
            raise TrackerError("tracker.auth", "rejected s3cret-key")
        return {"org": conn.org.strip("/"), "project": conn["project"].upper()}

    def states(self, conn, secret):
        return [{"name": "Doing", "color": "", "category": "x", "types": ["Task"], "suggested": "active"}]

    def fields(self, conn, secret):
        raise RuntimeError("leaked %s" % secret)

    def work_items(self, conn, secret, ids, request):
        return {i: {"title": "T%s" % i, "text": request.get("text")} for i in ids}

    def assigned_to_me(self, conn, secret, due_field, start, end, exclude):
        return [due_field, start, end, exclude]

    def detail(self, conn, secret, id):
        if id == "404":
            raise TrackerError("tracker.notFound")
        return {"id": id, "title": "Detail"}

    def set_state(self, conn, secret, id, state):
        return {"state": state, "changed": "2026-01-01"}


if __name__ == "__main__":
    plugin.run()
