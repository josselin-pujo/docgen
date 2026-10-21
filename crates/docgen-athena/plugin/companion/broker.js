"use strict";

// docgen-workspace — filesystem companion to the `docgen` foundation plugin.
//
// Requires the `docgen` plugin (mandatory dep): its `docgen_compose` tool is
// the renderer. This plugin only owns the filesystem edges — minting
// workspace files into blob refs (`fs.readBlob`, host-side harvest) and
// materializing blob refs back into files (`fs.writeBlob`, host-side write):
//
//   docgen_workspace      one-call path flow (mint → render → materialize)
//   docgen_file_to_blob   path → ref   (for chaining with docgen_compose)
//   docgen_blob_to_file   ref → path   (materialize any artifact)
//
// The one-call tool delegates the actual render by invoking the foundation's
// tool on the live table (`Athena.tool.invoke`). If that plugin is not
// active, the tool refuses with the manual 3-step chain spelled out — the
// model orchestrates across plugins, and this broker holds no component of
// its own (the foundation owns the wasm).

const MANIFEST =
  (typeof globalThis.__athenaManifest === "object" &&
    globalThis.__athenaManifest) ||
  {};
const CONFIG = parseConfig(MANIFEST.config);
const DEBUG = CONFIG.debug === true;

const FOUNDATION_TOOL = "docgen_compose";
const FORMATS = {
  render: ["pdf", "docx", "pptx"],
  diagram: ["svg", "png"],
};
const EXT = { pdf: "pdf", docx: "docx", pptx: "pptx", svg: "svg", png: "png" };
// The foundation's result text carries the artifact ref on its own line;
// both sides are ours, so this is a pinned contract (see the runbook).
const ARTIFACT_LINE = /artifact: (sha256:[0-9a-f]{64})/;

const SPEC_WORKSPACE = {
  name: "docgen_workspace",
  description:
    "Render a workspace file to PDF/DOCX/PPTX (or a diagram source to SVG/PNG) in one call: " +
    "the file paths are minted into blob refs, the render is delegated to the `docgen` " +
    "plugin, and the artifact is written back to the workspace. `assets` lists the images " +
    "the markdown references, as {name, path} (name = the URL used in the markdown; a plain " +
    "string works when they are identical). Prefer this over `docgen_compose` whenever the " +
    "source lives in the workspace.",
  parameters: {
    type: "object",
    properties: {
      action: {
        type: "string",
        enum: ["render", "diagram"],
        description: "render: markdown → document; diagram: source file → image.",
      },
      format: {
        type: "string",
        enum: ["pdf", "docx", "pptx", "svg", "png"],
        description: "pdf/docx/pptx for render; svg/png for diagram.",
      },
      source: {
        type: "string",
        description: "Workspace-relative path of the source file.",
      },
      assets: {
        type: "array",
        description:
          "Images referenced by the markdown. Each entry: {name, path} — name is the URL " +
          "in the markdown, path is the workspace file; a bare string covers both when equal.",
        items: {
          type: ["object", "string"],
          properties: {
            name: { type: "string" },
            path: { type: "string" },
          },
          required: ["path"],
        },
      },
      out: {
        type: "string",
        description:
          "Workspace-relative output path; defaults to the source name with the new extension.",
      },
      kind: {
        type: "string",
        enum: ["mermaid", "excalidraw"],
        description: "Diagram family; inferred when omitted.",
      },
      surface: {
        type: "boolean",
        description: "Surface the artifact as a media card (default true).",
      },
    },
    required: ["action", "format", "source"],
  },
};

const SPEC_MINT = {
  name: "docgen_file_to_blob",
  description:
    "Mint a workspace file into a blob ref (host-side harvest, no bytes in the tool call). " +
    "Use the returned digest as `source.ref` or an `assets[].ref` for `docgen_compose` — " +
    "the ref chain also feeds other tools.",
  parameters: {
    type: "object",
    properties: {
      path: {
        type: "string",
        description: "Workspace-relative path of the file to mint.",
      },
    },
    required: ["path"],
  },
};

const SPEC_WRITE = {
  name: "docgen_blob_to_file",
  description:
    "Materialize a blob ref (artifact digest) into a workspace file (host-side write). " +
    "The complement of `docgen_file_to_blob`.",
  parameters: {
    type: "object",
    properties: {
      ref: {
        type: "string",
        description: "Blob digest (sha256:…) to write.",
      },
      path: {
        type: "string",
        description: "Workspace-relative destination path.",
      },
    },
    required: ["ref", "path"],
  },
};

function handler(req) {
  if (!req || typeof req !== "object") return { ok: false, output: "bad bridge request" };
  switch (req.op) {
    case "specs":
      return { specs: [SPEC_WORKSPACE, SPEC_MINT, SPEC_WRITE] };
    case "sections":
      return { sections: [] };
    case "invoke":
      switch (req.tool) {
        case "docgen_workspace":
          return workspace(req);
        case "docgen_file_to_blob":
          return mint(req);
        case "docgen_blob_to_file":
          return materialize(req);
        default:
          return { ok: false, output: "unknown tool: " + String(req.tool) };
      }
    default:
      return { ok: false, output: "unknown op: " + String(req.op) };
  }
}

function workspace(req) {
  const args = req.args && typeof req.args === "object" ? req.args : {};
  const action = str(args.action);
  const format = str(args.format);
  const source = str(args.source);

  if (action !== "render" && action !== "diagram") {
    return fail('action must be "render" or "diagram"');
  }
  if (FORMATS[action].indexOf(format) === -1) {
    return fail(action + ": format must be one of " + FORMATS[action].join(", "));
  }
  if (!source) {
    return fail("pass `source` — the workspace-relative path of the source file");
  }
  const out = str(args.out) || defaultOut(source, format);

  // Validate the assets shape before any IO.
  const assetSpecs = [];
  if (args.assets !== undefined) {
    if (!Array.isArray(args.assets)) {
      return fail("assets must be an array of {name, path} (or plain path strings)");
    }
    for (const a of args.assets) {
      const name = typeof a === "string" ? a : str(a && a.name);
      const apath = typeof a === "string" ? a : str(a && a.path);
      if (apath === "") {
        return fail("each asset needs a path ({name, path}, or a plain path string)");
      }
      assetSpecs.push({ name: name === "" ? apath : name, path: apath });
    }
  }

  // Mint the source (host-side harvest; bytes never transit this isolate).
  let ref;
  try {
    ref = Athena.fs.readBlob(source);
  } catch (e) {
    return fail("reading " + source + ": " + prose(e));
  }
  if (typeof ref !== "string" || ref.length === 0) {
    return fail("reading " + source + " produced no blob ref");
  }

  // Mint the declared assets; a failed one degrades to a placeholder with a
  // note (the render still delivers the rest).
  const assets = [];
  const notes = [];
  for (const spec of assetSpecs) {
    try {
      const aref = Athena.fs.readBlob(spec.path);
      if (typeof aref !== "string" || aref.length === 0) {
        notes.push("asset " + spec.path + ": no blob ref produced — rendered as a placeholder");
        continue;
      }
      assets.push({ name: spec.name, ref: aref });
    } catch (e) {
      notes.push("asset " + spec.path + ": " + prose(e) + " — rendered as a placeholder");
    }
  }

  // Delegate the render to the foundation plugin's tool on the live table.
  const nested = {
    action: action,
    format: format,
    kind: str(args.kind),
    source: { ref: ref },
  };
  if (assets.length) nested.assets = assets;
  if (typeof args.surface === "boolean") nested.surface = args.surface;

  let reply;
  try {
    reply = Athena.tool.invoke(String(req.callKey || "docgen_workspace") + ":render", FOUNDATION_TOOL, nested);
  } catch (e) {
    return fail(
      "the docgen foundation plugin is not available (" +
        prose(e) +
        ") — activate `docgen`, or chain manually: docgen_file_to_blob → docgen_compose → docgen_blob_to_file"
    );
  }
  if (DEBUG) console.error("[docgen-workspace] render reply: " + JSON.stringify(reply));
  if (!reply || !reply.ok) {
    return fail("render failed:\n" + (reply ? reply.output : "no reply from " + FOUNDATION_TOOL));
  }

  // The foundation's text carries the artifact line (pinned contract).
  const m = ARTIFACT_LINE.exec(String(reply.output || ""));
  if (!m) {
    return fail("could not read the artifact ref from the render result:\n" + reply.output);
  }

  try {
    Athena.fs.writeBlob(out, m[1]);
  } catch (e) {
    return fail(
      "rendered, but could not write " + out + ": " + prose(e) + "\nartifact: " + m[1]
    );
  }

  let output = "wrote " + out + " (" + format + ")\nartifact: " + m[1];
  const tail = String(reply.output || "");
  const idx = tail.indexOf("artifact: ");
  const rest = idx >= 0 ? tail.slice(idx + "artifact: ".length).split("\n").slice(1) : [];
  for (const line of rest) {
    if (line.startsWith("warnings:") || line.startsWith("  - ")) output += "\n" + line;
    else if (line.startsWith("surfaced")) output += "\n" + line;
  }
  for (const n of notes) output += "\n  - " + n;
  // The media side-channel: the foundation's reply carries the surfaced
  // reference (digest/type/name); spreading it here lets the harness
  // journal the media.put from THIS tool's run result — the card lands
  // attributed to the tool the model actually called.
  const out2 = { ok: true, output: output };
  if (reply.media) out2.media = reply.media;
  return out2;
}

function mint(req) {
  const args = req.args && typeof req.args === "object" ? req.args : {};
  const path = str(args.path);
  if (!path) return fail("pass `path` — the workspace file to mint");
  let ref;
  try {
    ref = Athena.fs.readBlob(path);
  } catch (e) {
    return fail("reading " + path + ": " + prose(e));
  }
  if (typeof ref !== "string" || ref.length === 0) {
    return fail("reading " + path + " produced no blob ref");
  }
  return { ok: true, output: ref };
}

function materialize(req) {
  const args = req.args && typeof req.args === "object" ? req.args : {};
  const ref = str(args.ref);
  const path = str(args.path);
  if (!ref) return fail("pass `ref` — the blob digest to write");
  if (!path) return fail("pass `path` — the destination file");
  try {
    Athena.fs.writeBlob(path, ref);
  } catch (e) {
    return fail("writing " + path + ": " + prose(e));
  }
  return { ok: true, output: "wrote " + path };
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

function defaultOut(source, format) {
  const ext = EXT[format];
  const base = baseName(source);
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.substring(0, dot) : base;
  return source.substring(0, source.length - base.length) + stem + "." + ext;
}

Athena.serve(handler);
