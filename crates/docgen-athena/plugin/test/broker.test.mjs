// Node smoke tests for the docgen FOUNDATION broker (blob-native, no fs).
// The host-injected `Athena` global is faked; the broker source is loaded as
// a script with the stub in scope, and its handler is driven directly.
// Run: `node --test crates/docgen-athena/plugin/test/broker.test.mjs`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = readFileSync(path.join(here, "..", "broker.js"), "utf8");

const REF = "sha256:" + "a".repeat(64);
const ARTIFACT = "sha256:" + "b".repeat(64);

// Cross-realm-safe plain-data comparison.
const norm = (x) => JSON.parse(JSON.stringify(x));

function loadBroker(opts = {}) {
  const calls = { put: [], invoke: [], toolInvoke: [] };
  const stubs = opts.stubs || {};

  globalThis.Athena = {
    serve(h) {
      loadBroker.handler = h;
    },
    blob: {
      put(b) {
        calls.put.push(b);
        if (stubs.blobPut) return stubs.blobPut(b);
        return "sha256:inline";
      },
    },
    wasm: {
      invoke(tool, input) {
        calls.invoke.push([tool, input]);
        if (stubs.wasmInvoke) return stubs.wasmInvoke(tool, input);
        return { ok: { artifact: { ref: ARTIFACT }, warnings: [] } };
      },
    },
    tool: {
      invoke(key, tool, args) {
        calls.toolInvoke.push([key, tool, args]);
        if (stubs.toolInvoke) return stubs.toolInvoke(key, tool, args);
        return { ok: true, output: "ok" };
      },
    },
  };
  globalThis.__athenaManifest = opts.manifest || { config: {} };

  // eslint-disable-next-line no-new-func
  new Function(source)();
  return { handler: loadBroker.handler, calls };
}

const invoke = (handler, args, callKey = "c1") =>
  handler({ op: "invoke", tool: "docgen_compose", callKey, args });

test("specs: exactly one docgen_compose ToolSpec", () => {
  const { handler } = loadBroker();
  const r = handler({ op: "specs" });
  assert.equal(r.specs.length, 1);
  const s = r.specs[0];
  assert.equal(s.name, "docgen_compose");
  assert.equal(typeof s.description, "string");
  assert.deepEqual(s.parameters.required, ["action", "format"]);
  assert.ok(s.parameters.properties.content && s.parameters.properties.source);
  assert.ok(s.parameters.properties.assets);
});

test("sections: empty list; unknown ops answer honestly", () => {
  const { handler } = loadBroker();
  assert.deepEqual(handler({ op: "sections" }), { sections: [] });
  assert.equal(handler({ op: "wat" }).output, "unknown op: wat");
});

test("inline content: blob.put once, ref rides into the component", () => {
  const { handler, calls } = loadBroker();
  const r = invoke(handler, { action: "render", format: "pdf", content: "# Hi" });
  assert.equal(r.ok, true);
  assert.match(r.output, /^rendered pdf/);
  assert.match(r.output, new RegExp(ARTIFACT));
  assert.match(r.output, /surfaced as a media card/);
  assert.equal(calls.put.length, 1);
  assert.equal(Buffer.from(calls.put[0]).toString(), "# Hi");
  const [tool, input] = calls.invoke[0];
  assert.equal(tool, "docgen_compose");
  assert.deepEqual(norm(input), {
    action: "render",
    format: "pdf",
    kind: "",
    content: { ref: "sha256:inline" },
    assets: [],
  });
  // The de-hex-era surfacing: the media rides the reply's side-channel
  // (no load_artifact delegation — toolInvoke stays untouched).
  assert.equal(calls.toolInvoke.length, 0);
  assert.deepEqual(norm(r.media), {
    digest: ARTIFACT,
    mediaType: "application/pdf",
    bytes: 0,
    name: "docgen_compose-pdf",
    path: "",
  });
});

test("every format surfaces with its precise media type", () => {
  const cases = [
    ["render", "pdf", "application/pdf"],
    [
      "render",
      "docx",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ],
    [
      "render",
      "pptx",
      "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ],
    ["diagram", "svg", "image/svg+xml"],
    ["diagram", "png", "image/png"],
  ];
  for (const [action, format, mime] of cases) {
    const { handler, calls } = loadBroker();
    const r = invoke(handler, { action, format, content: "x" });
    assert.equal(r.ok, true, action + "/" + format);
    assert.equal(calls.toolInvoke.length, 0, action + "/" + format);
    assert.equal(r.media.mediaType, mime, action + "/" + format);
    assert.equal(r.media.digest, ARTIFACT, action + "/" + format);
  }
});

test("source ref: passed through without minting, no fs anywhere", () => {
  const { handler, calls } = loadBroker();
  const r = invoke(handler, { action: "render", format: "docx", source: { ref: REF } });
  assert.equal(r.ok, true);
  assert.equal(calls.put.length, 0);
  assert.deepEqual(norm(calls.invoke[0][1].content), { ref: REF });
});

test("source ref validation is strict", () => {
  const { handler } = loadBroker();
  assert.match(invoke(handler, { action: "render", format: "pdf", source: { ref: "nope" } }).output, /blob digest/);
  assert.match(invoke(handler, { action: "render", format: "pdf", source: {} }).output, /blob digest/);
});

test("exactly one of content / source", () => {
  const { handler } = loadBroker();
  assert.match(invoke(handler, { action: "render", format: "pdf" }).output, /exactly one of/);
  assert.match(
    invoke(handler, { action: "render", format: "pdf", content: "x", source: { ref: REF } }).output,
    /not both/,
  );
});

test("assets map to the component's {name, image:{ref}} list", () => {
  const { handler, calls } = loadBroker();
  const r = invoke(handler, {
    action: "render",
    format: "pdf",
    content: "![x](img/x.png)",
    assets: [{ name: "img/x.png", ref: REF }],
  });
  assert.equal(r.ok, true);
  assert.deepEqual(norm(calls.invoke[0][1].assets), [
    { name: "img/x.png", image: { ref: REF } },
  ]);
});

test("asset validation is strict", () => {
  const { handler } = loadBroker();
  assert.match(
    invoke(handler, { action: "render", format: "pdf", content: "x", assets: "nope" }).output,
    /array of/,
  );
  assert.match(
    invoke(handler, { action: "render", format: "pdf", content: "x", assets: [{ name: "a", ref: "x" }] }).output,
    /each asset needs/,
  );
});

test("component warnings ride the output", () => {
  const { handler } = loadBroker({
    stubs: {
      wasmInvoke: () => ({
        ok: { artifact: { ref: ARTIFACT }, warnings: ["image img/gone.png: no bytes provided — rendered as a visible placeholder"] },
      }),
    },
  });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, true);
  assert.match(r.output, /warnings:/);
  assert.match(r.output, /img\/gone\.png/);
});

test("argument refusals are honest prose", () => {
  const { handler } = loadBroker();
  assert.match(invoke(handler, { action: "paint", format: "pdf", content: "x" }).output, /render.*diagram/);
  assert.match(invoke(handler, { action: "render", format: "svg", content: "x" }).output, /pdf, docx, pptx/);
  assert.match(invoke(handler, { action: "diagram", format: "docx", content: "x" }).output, /svg, png/);
});

test("wasm error envelope becomes readable failure", () => {
  const { handler } = loadBroker({
    stubs: {
      wasmInvoke: () => ({ err: { origin: "host", kind: "timeout", message: "wall clock exceeded" } }),
    },
  });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, false);
  assert.match(r.output, /host\/timeout/);
  assert.match(r.output, /wall clock exceeded/);
});

test("surfacing rides the reply's media side-channel (no load_artifact delegation)", () => {
  // The de-hex-era design: the reply carries `media` and the harness
  // journals the media.put from its run result — surfacing needs no
  // capability here and cannot fail broker-side (a load_artifact
  // outage is irrelevant; nothing delegates to it).
  const { handler } = loadBroker({
    stubs: {
      toolInvoke: () => {
        throw new Error("load_artifact unavailable");
      },
    },
  });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, true);
  assert.match(r.output, /surfaced as a media card/);
  assert.equal(r.media.digest, "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
  assert.equal(r.media.mediaType, "application/pdf");
  assert.equal(r.media.name, "docgen_compose-pdf");
});

test("surface=false via config and via args skips surfacing", () => {
  const byConfig = loadBroker({ manifest: { config: { surface: false, debug: true } } });
  invoke(byConfig.handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(byConfig.calls.toolInvoke.length, 0);

  const byArgs = loadBroker();
  invoke(byArgs.handler, { action: "render", format: "pdf", content: "x", surface: false });
  assert.equal(byArgs.calls.toolInvoke.length, 0);
});

test("config may arrive as a JSON string (parse-once discipline)", () => {
  const { handler, calls } = loadBroker({ manifest: { config: '{"surface":false}' } });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, true);
  assert.equal(calls.toolInvoke.length, 0);
});
