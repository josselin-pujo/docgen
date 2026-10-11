"use strict";

// docgen — Athena plugin broker.
//
// One model-facing tool (`docgen_compose`) backed by the `docgen-athena`
// wasm component (the bundle's `components` row). The broker owns argument
// validation and fs delivery; the component is pure compute.
//
// The source rides into the component as a blob holder — `fs.readBlob`
// harvests a workspace file host-side, `blob.put` takes inline text once —
// so bytes never transit this isolate for files. The artifact rides out as
// a host-minted holder and is materialized with `fs.writeBlob`.

const MANIFEST =
  (typeof globalThis.__athenaManifest === "object" &&
    globalThis.__athenaManifest) ||
  {};
const CONFIG = parseConfig(MANIFEST.config);
const DEBUG = CONFIG.debug === true;
const SURFACE = CONFIG.surface !== false;

const TOOL = "docgen_compose";
const FORMATS = {
  render: ["pdf", "docx", "pptx"],
  diagram: ["svg", "png"],
};
const EXT = { pdf: "pdf", docx: "docx", pptx: "pptx", svg: "svg", png: "png" };

const SPEC = {
  name: TOOL,
  description:
    "Render a document or diagram in-process (no shell): markdown with YAML frontmatter " +
    "(```mermaid```/```excalidraw``` blocks render natively) → PDF/DOCX/PPTX; or a " +
    "mermaid/excalidraw source → SVG/PNG. Pass source (workspace file, preferred) or " +
    "content (inline text), not both. The result is a workspace file plus a transcript " +
    "artifact card.",
  parameters: {
    type: "object",
    properties: {
      action: {
        type: "string",
        enum: ["render", "diagram"],
        description:
          "render: markdown → document; diagram: mermaid/excalidraw source → image.",
      },
      format: {
        type: "string",
        enum: ["pdf", "docx", "pptx", "svg", "png"],
        description: "pdf/docx/pptx for render; svg/png for diagram.",
      },
      source: {
        type: "string",
        description:
          "Workspace-relative path of the source file (preferred for large sources).",
      },
      content: {
        type: "string",
        description: "Inline source text — alternative to source, for short sources.",
      },
      kind: {
        type: "string",
        enum: ["mermaid", "excalidraw"],
        description:
          "Diagram family; inferred from the source extension/content when omitted.",
      },
      out: {
        type: "string",
        description:
          "Workspace-relative output path; defaults to the source name with the new extension.",
      },
    },
    required: ["action", "format"],
  },
};

function handler(req) {
  if (!req || typeof req !== "object") return { ok: false, output: "bad bridge request" };
  switch (req.op) {
    case "specs":
      return { specs: [SPEC] };
    case "sections":
      // No prompt sections to add; symmetric-with-specs default.
      return { sections: [] };
    case "invoke":
      return invoke(req);
    default:
      return { ok: false, output: "unknown op: " + String(req.op) };
  }
}

function invoke(req) {
  const args = req.args && typeof req.args === "object" ? req.args : {};
  const action = str(args.action);
  const format = str(args.format);
  const source = str(args.source);
  const content = str(args.content);
  const kindArg = str(args.kind);
  const outArg = str(args.out);

  if (action !== "render" && action !== "diagram") {
    return fail('action must be "render" or "diagram"');
  }
  if (FORMATS[action].indexOf(format) === -1) {
    return fail(action + ": format must be one of " + FORMATS[action].join(", "));
  }
  if (!source && !content) {
    return fail("pass exactly one of `source` (workspace file) or `content` (inline text)");
  }
  if (source && content) {
    return fail("pass exactly one of `source` or `content` — not both");
  }

  // Resolve the source to a blob ref (host-side for files).
  let ref;
  try {
    if (source) {
      ref = Athena.fs.readBlob(source);
    } else {
      ref = Athena.blob.put(new TextEncoder().encode(content));
    }
  } catch (e) {
    return fail("reading the source failed: " + prose(e));
  }
  if (typeof ref !== "string" || ref.length === 0) {
    return fail("reading the source produced no blob ref");
  }

  // Diagram family: explicit `kind` wins; otherwise extension, then sniff
  // (mirrors the CLI's detect: excalidraw = .excalidraw/.json extension or
  // a JSON object mentioning "elements"/"type": "excalidraw").
  let kind = "";
  if (action === "diagram") {
    kind = kindArg || inferKind(source, content);
    if (kind !== "mermaid" && kind !== "excalidraw") {
      return fail('diagram: pass kind "mermaid" or "excalidraw" (could not classify the source)');
    }
  }

  // Invoke the component: the host fills `content.data` from the ref and
  // mints the out-holder (`artifact.ref`).
  let reply;
  try {
    reply = Athena.wasm.invoke(TOOL, {
      action: action,
      format: format,
      kind: kind,
      content: { ref: ref },
    });
  } catch (e) {
    return fail("docgen wasm invoke failed: " + prose(e));
  }
  if (DEBUG) console.error("[docgen] wasm reply: " + JSON.stringify(reply));
  if (!reply || !reply.ok) {
    return fail(describeEnvelope(reply));
  }
  const artifact = reply.ok.artifact;
  const artifactRef = artifact && artifact.ref;
  if (typeof artifactRef !== "string" || artifactRef.length === 0) {
    return fail("component returned no artifact ref");
  }

  const out = outArg || defaultOut(source, action, format);
  try {
    Athena.fs.writeBlob(out, artifactRef);
  } catch (e) {
    return {
      ok: false,
      output:
        "rendered, but could not write " +
        out +
        ": " +
        prose(e) +
        "\nartifact: " +
        artifactRef,
    };
  }

  // Best-effort: surface the artifact to the transcript from the existing
  // digest (a failure here must never fail the tool).
  if (SURFACE) {
    try {
      Athena.tool.invoke(String(req.callKey || TOOL) + ":surface", "load_artifact", {
        digest: artifactRef,
      });
    } catch (e) {
      console.error("[docgen] artifact surfacing failed: " + prose(e));
    }
  }

  return {
    ok: true,
    output: "wrote " + out + " (" + format + ")\nartifact: " + artifactRef,
  };
}

// --- helpers ---------------------------------------------------------------

function fail(output) {
  return { ok: false, output: output };
}

function str(v) {
  return typeof v === "string" ? v : "";
}

function prose(e) {
  return e && e.message ? String(e.message) : String(e);
}

function parseConfig(config) {
  if (typeof config === "string") {
    try {
      return JSON.parse(config) || {};
    } catch (_) {
      return {};
    }
  }
  return config && typeof config === "object" ? config : {};
}

function baseName(p) {
  return p.substring(p.lastIndexOf("/") + 1);
}

function inferKind(source, content) {
  if (source) {
    const base = baseName(source);
    const dot = base.lastIndexOf(".");
    const ext = (dot > 0 ? base.substring(dot + 1) : "").toLowerCase();
    if (ext === "excalidraw" || ext === "json") return "excalidraw";
    if (ext === "mmd" || ext === "mermaid") return "mermaid";
  }
  let text = content;
  if (!text && source) {
    try {
      text = new TextDecoder().decode(Athena.fs.read(source));
    } catch (_) {
      text = "";
    }
  }
  if (!text) return "";
  const trimmed = text.replace(/^\s+/, "");
  if (
    trimmed.startsWith("{") &&
    (text.indexOf('"elements"') !== -1 || text.indexOf('"type": "excalidraw"') !== -1)
  ) {
    return "excalidraw";
  }
  return "mermaid";
}

function defaultOut(source, action, format) {
  const ext = EXT[format];
  if (source) {
    const base = baseName(source);
    const dot = base.lastIndexOf(".");
    const stem = dot > 0 ? base.substring(0, dot) : base;
    return source.substring(0, source.length - base.length) + stem + "." + ext;
  }
  return (action === "render" ? "document." : "diagram.") + ext;
}

function describeEnvelope(reply) {
  if (reply && reply.err) {
    const e = reply.err;
    const parts = [];
    if (e.origin) parts.push(e.origin);
    if (e.kind) parts.push(e.kind);
    const head = parts.length ? " (" + parts.join("/") + ")" : "";
    return "docgen failed" + head + ": " + (e.message || JSON.stringify(e));
  }
  return "docgen returned an unexpected envelope: " + JSON.stringify(reply);
}

Athena.serve(handler);
