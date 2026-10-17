"use strict";

// docgen — Athena plugin broker (foundation: blob-native, no filesystem).
//
// One model-facing tool (`docgen_compose`) backed by the `docgen-athena`
// wasm component (the bundle's `components` row). The contract is refs-only:
// sources ride in as host-minted blob holders (inline text is blob-put once)
// and the artifact rides out as a host-minted ref, surfaced to the transcript
// as a media card. Nothing here touches a filesystem — fs-backed
// conveniences (path sources, materialization, image harvesting) live in the
// `docgen-workspace` companion plugin, which depends on this one and
// delegates the actual render here.

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
// The artifact's media type, claimed at surfacing time: the host's media
// sniffer only knows pdf/webp/jpeg/gif/png, so OOXML documents and SVG
// diagrams surface as typeless blobs without this — no transcript card.
// `load_artifact`'s `media_type` is the caller's precise claim; the blob
// store stamps it (first non-null wins) and serves it back.
const MIME = {
  pdf: "application/pdf",
  docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  svg: "image/svg+xml",
  png: "image/png",
};
const DIGEST = /^sha256:[0-9a-f]{64}$/;

const SPEC = {
  name: TOOL,
  description:
    "Render a document or diagram in-process (no shell, no filesystem needed): " +
    "markdown with YAML frontmatter (```mermaid```/```excalidraw``` blocks render natively) " +
    "→ PDF/DOCX/PPTX; or a mermaid/excalidraw source → SVG/PNG. Pass `content` (inline text) " +
    "or `source` ({ref}) — the source as a blob digest, e.g. another tool's artifact. Image " +
    "URLs in the markdown resolve against `assets` (name = the exact URL, ref = blob digest). " +
    "The artifact comes back as a workspace-independent blob ref, surfaced as a transcript card.",
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
      content: {
        type: "string",
        description: "Inline source text — alternative to source, for short sources.",
      },
      source: {
        type: "object",
        properties: {
          ref: {
            type: "string",
            description: "Blob digest (sha256:…) of the source bytes.",
          },
        },
        required: ["ref"],
        description:
          "The source as a blob ref: a digest from `docgen_file_to_blob` (workspace " +
          "plugin) or another tool's artifact — bytes never transit the tool call.",
      },
      assets: {
        type: "array",
        description:
          "Inline images for render: name is the exact URL the markdown uses " +
          "(e.g. \"img/shot.png\"); ref is the image bytes' blob digest.",
        items: {
          type: "object",
          properties: {
            name: { type: "string", description: "Markdown image URL." },
            ref: { type: "string", description: "Blob digest (sha256:…)." },
          },
          required: ["name", "ref"],
        },
      },
      kind: {
        type: "string",
        enum: ["mermaid", "excalidraw"],
        description: "Diagram family; inferred from the source when omitted.",
      },
      surface: {
        type: "boolean",
        description:
          "Surface the artifact as a media card (default true; config can flip the default).",
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
  const content = str(args.content);
  const kind = str(args.kind);
  const surface = typeof args.surface === "boolean" ? args.surface : SURFACE;

  if (action !== "render" && action !== "diagram") {
    return fail('action must be "render" or "diagram"');
  }
  if (FORMATS[action].indexOf(format) === -1) {
    return fail(action + ": format must be one of " + FORMATS[action].join(", "));
  }
  if (!content && !args.source) {
    return fail("pass exactly one of `content` (inline text) or `source` ({ref})");
  }
  if (content && args.source) {
    return fail("pass exactly one of `content` or `source` — not both");
  }

  // The source resolves to a blob ref: harvest inline text once, or pass an
  // existing digest straight through (the host derefs it into the component).
  let ref;
  if (content) {
    ref = Athena.blob.put(new TextEncoder().encode(content));
  } else {
    ref = args.source && typeof args.source === "object" ? str(args.source.ref) : "";
    if (!DIGEST.test(ref)) {
      return fail("source.ref must be a blob digest (sha256:…)");
    }
  }
  if (typeof ref !== "string" || ref.length === 0) {
    return fail("the source produced no blob ref");
  }

  // Assets: named refs; names are the markdown URLs.
  const assets = [];
  if (args.assets !== undefined) {
    if (!Array.isArray(args.assets)) {
      return fail("assets must be an array of {name, ref}");
    }
    for (const a of args.assets) {
      const name = str(a && a.name);
      const aref = str(a && a.ref);
      if (name === "" || !DIGEST.test(aref)) {
        return fail('each asset needs {name: "img/…", ref: "sha256:…"}');
      }
      assets.push({ name: name, image: { ref: aref } });
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
      assets: assets,
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
  const warnings = Array.isArray(reply.ok.warnings) ? reply.ok.warnings : [];

  // Best-effort: surface the artifact to the transcript from the existing
  // digest (a failure here must never fail the tool).
  let surfaced = false;
  if (surface) {
    try {
      Athena.tool.invoke(String(req.callKey || TOOL) + ":surface", "load_artifact", {
        digest: artifactRef,
        media_type: MIME[format],
      });
      surfaced = true;
    } catch (e) {
      console.error("[docgen] artifact surfacing failed: " + prose(e));
    }
  }

  let output = "rendered " + format + "\nartifact: " + artifactRef;
  if (surfaced) output += "\nsurfaced as a media card";
  if (warnings.length) {
    output += "\nwarnings:\n  - " + warnings.join("\n  - ");
  }
  return { ok: true, output: output };
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
