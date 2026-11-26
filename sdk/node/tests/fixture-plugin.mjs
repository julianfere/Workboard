// Plugin used by the SDK tests: exercises every part of the SDK.
import {
  HostError, Plugin, TrackerError, button, close, form, info, result, select, session, text,
} from "../workboard-plugin.mjs";

const plugin = new Plugin();

plugin.action("greet", {
  async open(ctx) {
    console.log("stray log goes to stderr");
    const item = ctx.item ? await ctx.host.item(ctx.item.name) : { title: "-" };
    let denied = null;
    try {
      await ctx.host.secretGet("token");
    } catch (e) {
      if (e instanceof HostError) denied = e.code;
    }
    return form(
      `${item.title} · ${ctx.lang}`,
      [
        text("name", "Name", { required: true }),
        select("flow", "Flow", [["a", "A"], "b"], { refresh: true }),
        info("Some help"),
      ],
      {
        buttons: [["more", "More"], button("greet", "Greet", { primary: true, confirm: "Sure?" })],
        state: { denied, placement: ctx.placement, dir: ctx.pluginDir },
      },
    );
  },
  async submit(ctx, btn, values, state) {
    if (btn === "session") return session(ctx.items.map((i) => i.name), { prompt: "Hi", mode: "plan" });
    if (btn === "close") return close("Saved");
    await ctx.host.updateItem(ctx.item.name, { notes: `greeted ${values.name}` });
    return result("Hello", `¡Hola, ${values.name}!`, {
      steps: [["Saved", true]],
      links: [["Open", "https://example.com"]],
      state,
    });
  },
});

plugin.action("boom", {
  open() {
    throw new TypeError("boom");
  },
});

plugin.action("hosterr", {
  async open(ctx) {
    await ctx.host.readFile("x.txt");
  },
});

plugin.tracker("fake", {
  connect(conn, secret) {
    if (secret !== "s3cret-key") throw new TrackerError("tracker.auth", "rejected s3cret-key");
    return { org: conn.org.replace(/\/$/, ""), project: conn.project.toUpperCase() };
  },
  states: () => [{ name: "Doing", color: "", category: "x", types: ["Task"], suggested: "active" }],
  fields(conn, secret) {
    throw new Error(`leaked ${secret}`);
  },
  workItems: async (conn, secret, ids, request) =>
    Object.fromEntries(ids.map((i) => [i, { title: `T${i}`, text: request.text }])),
  assignedToMe: (conn, secret, dueField, start, end, exclude) => [dueField, start, end, exclude],
  detail(conn, secret, id) {
    if (id === "404") throw new TrackerError("tracker.notFound");
    return { id, title: "Detail" };
  },
  setState: (conn, secret, id, state) => ({ state, changed: "2026-01-01" }),
});

plugin.run();
