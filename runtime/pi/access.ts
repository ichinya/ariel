import { Type } from "@earendil-works/pi-ai";
export default function (pi) {
  pi.registerTool({
    name: "ariel_ask_user",
    label: "Ask the user",
    description: "Ask a clarification through Ariel and wait for the answer. This does not grant filesystem permissions.",
    parameters: Type.Object({method: Type.Union([Type.Literal("input"), Type.Literal("select"), Type.Literal("confirm")]), title: Type.String(), prompt: Type.String(), options: Type.Optional(Type.Array(Type.String()))}),
    async execute(_id, params, _signal, _update, ctx) {
      const answer = params.method === "confirm" ? await ctx.ui.confirm(params.title, params.prompt)
        : params.method === "select" ? await ctx.ui.select(params.title + "\n" + params.prompt, params.options || [])
        : await ctx.ui.input(params.title, params.prompt);
      return { content: [{ type: "text", text: JSON.stringify({answer: answer ?? null}) }], details: {} };
    },
  });
  pi.registerTool({
    name: "ariel_request_path",
    label: "Request host path",
    description: "Ask the human owner to mount an additional host path. Supply an absolute host path and read or write access. Approval stops this attempt; use the granted /grants path on the next user turn. It never grants access automatically.",
    parameters: Type.Object({ path: Type.String(), access: Type.Union([Type.Literal("read"), Type.Literal("write")]) }),
    async execute(_id, params, _signal, _onUpdate, ctx) {
      const allowed = await ctx.ui.confirm("ARIEL_PATH_ACCESS", JSON.stringify(params));
      return { content: [{ type: "text", text: allowed ? "Access approved for a new runtime generation." : "Access not granted in this attempt." }] };
    },
  });
}
