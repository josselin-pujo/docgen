// Node smoke tests for the docgen-workspace COMPANION broker (fs edges +
// delegation to the foundation tool). Stubs fake `Athena` (fs, tool.invoke);
// the broker source is loaded as a script with the stubs in scope.
// Run: `node --test crates/docgen-athena/plugin/test/workspace.test.mjs`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = readFileSync(path.join(here, "..", "companion", "broker.js"), "utf8");

const REF_SRC = "sha256:" + "1".repeat(64);
const REF_IMG = "sha256:" + "2".repeat(64);
const ARTIFACT = "sha256:" + "3".repeat(64);
const FOUNDATION_OK = {
  ok: true,
  output:
    "rendered pdf\nartifact: " +
    ARTIFACT +
    "\nsurfaced as a media card",
};

const norm = (x) => JSON.parse(JSON.stringify(x));

function loadBroker(opts = {}) {
  const calls = { readBlob: [], writeBlob: [], toolInvoke: [] };
  const stubs = opts.stubs || {};
  const blobs = stubs.blobs || {};

  globalThis.Athena = {
    serve(h) {
      loadBroker.handler = h;
    },
    fs: {
      readBlob(p) {
        calls.readBlob.push(p);
        if (stubs.readBlob) return stubs.readBlob(p);
        if (p in blobs) return blobs[p];
        throw new Error('no such workspace file: "' + p + '"');
      },
      writeBlob(p, d) {
        calls.writeBlob.push([p, d]);
        if (stubs.writeBlob) return stubs.writeBlob(p, d);
      },
    },
    tool: {
      invoke(key, tool, args) {
        calls.toolInvoke.push([key, tool, args]);
        if (stubs.toolInvoke) return stubs.toolInvoke(key, tool, args);
        return FOUNDATION_OK;
      },
    },
  };
  globalThis.__athenaManifest = opts.manifest || { config: {} };

  // eslint-disable-next-line no-new-func
  new Function(source)();
  return { handler: loadBroker.handler, calls };
}

const invoke = (handler, tool, args, callKey = "c1") =>
  handler({ op: "invoke", tool, callKey, args });

const BLOBS = { "docs/arch.md": REF_SRC, "docs/img/logo.png": REF_IMG };

test("specs: the three companion tools", () => {
  const { handler } = loadBroker();
  const r = handler({ op: "specs" });
  assert.deepEqual(
    r.specs.map((s) => s.name),
    ["docgen_workspace", "docgen_file_to_blob", "docgen_blob_to_file"],
  );
  const ws = r.specs[0];
  assert.deepEqual(ws.parameters.required, ["action", "format", "source"]);
});

test("sections: empty list; unknown op and unknown tool answer honestly", () => {
  const { handler } = loadBroker();
  assert.deepEqual(handler({ op: "sections" }), { sections: [] });
  assert.equal(handler({ op: "wat" }).output, "unknown op: wat");
  assert.match(invoke(handler, "docgen_nope", {}).output, /unknown tool/);
});

test("workspace: mints source + assets, delegates, materializes, reports", () => {
  const { handler, calls } = loadBroker({ stubs: { blobs: BLOBS } });
  const r = invoke(handler, "docgen_workspace", {
    action: "render",
    format: "pdf",
    source: "docs/arch.md",
    assets: [{ name: "img/logo.png", path: "docs/img/logo.png" }],
  });
  assert.equal(r.ok, true);
  assert.match(r.output, /^wrote docs\/arch\.pdf \(pdf\)/);
  assert.match(r.output, new RegExp(ARTIFACT));
  assert.match(r.output, /surfaced as a media card/);

  assert.deepEqual(calls.readBlob, ["docs/arch.md", "docs/img/logo.png"]);
  const [key, tool, args] = calls.toolInvoke[0];
  assert.equal(key, "c1:render");
  assert.equal(tool, "docgen_compose");
  assert.deepEqual(norm(args), {
    action: "render",
    format: "pdf",
    kind: "",
    source: { ref: REF_SRC },
    assets: [{ name: "img/logo.png", ref: REF_IMG }],
  });
  assert.deepEqual(norm(calls.writeBlob), [["docs/arch.pdf", ARTIFACT]]);
});

test("workspace: explicit out, plain-string assets, surface passthrough", () => {
  const { handler, calls } = loadBroker({ stubs: { blobs: { "a.md": REF_SRC } } });
  const r = invoke(handler, "docgen_workspace", {
    action: "render",
    format: "docx",
    source: "a.md",
    out: "build/a.docx",
    assets: ["a.md"],
    surface: false,
  });
  assert.equal(r.ok, true);
  assert.match(r.output, /^wrote build\/a\.docx \(docx\)/);
  const args = calls.toolInvoke[0][2];
  assert.equal(args.surface, false);
  assert.deepEqual(norm(args.assets), [{ name: "a.md", ref: REF_SRC }]);
});

test("workspace: a failed asset read degrades to a note, render continues", () => {
  const { handler, calls } = loadBroker({ stubs: { blobs: { "a.md": REF_SRC } } });
  const r = invoke(handler, "docgen_workspace", {
    action: "render",
    format: "pdf",
    source: "a.md",
    assets: [{ name: "img/gone.png", path: "img/gone.png" }],
  });
  assert.equal(r.ok, true);
  assert.match(r.output, /asset img\/gone\.png: .*placeholder/);
  assert.equal(calls.toolInvoke[0][2].assets, undefined);
});

test("workspace: foundation missing refuses with the manual chain", () => {
  const { handler } = loadBroker({
    stubs: {
      blobs: { "a.md": REF_SRC },
      toolInvoke: () => {
        throw new Error("no tool named docgen_compose");
      },
    },
  });
  const r = invoke(handler, "docgen_workspace", { action: "render", format: "pdf", source: "a.md" });
  assert.equal(r.ok, false);
  assert.match(r.output, /not available/);
  assert.match(r.output, /docgen_file_to_blob → docgen_compose → docgen_blob_to_file/);
  assert.equal(r.output.indexOf("wrote"), -1);
});

test("workspace: foundation failure and missing artifact line are readable", () => {
  const failed = loadBroker({
    stubs: {
      blobs: { "a.md": REF_SRC },
      toolInvoke: () => ({ ok: false, output: "render failed: bad mermaid" }),
    },
  });
  let r = invoke(failed.handler, "docgen_workspace", { action: "render", format: "pdf", source: "a.md" });
  assert.equal(r.ok, false);
  assert.match(r.output, /bad mermaid/);

  const noLine = loadBroker({
    stubs: {
      blobs: { "a.md": REF_SRC },
      toolInvoke: () => ({ ok: true, output: "rendered pdf" }),
    },
  });
  r = invoke(noLine.handler, "docgen_workspace", { action: "render", format: "pdf", source: "a.md" });
  assert.equal(r.ok, false);
  assert.match(r.output, /could not read the artifact/);
});

test("workspace: writeBlob refusal keeps the digest", () => {
  const { handler } = loadBroker({
    stubs: {
      blobs: { "a.md": REF_SRC },
      writeBlob: () => {
        throw new Error("fs.write refused: missing grant fs.write=docgen-workspace");
      },
    },
  });
  const r = invoke(handler, "docgen_workspace", { action: "render", format: "pdf", source: "a.md" });
  assert.equal(r.ok, false);
  assert.match(r.output, /could not write a\.pdf/);
  assert.match(r.output, /fs\.write refused/);
  assert.match(r.output, new RegExp(ARTIFACT));
});

test("workspace: source read refusal fails with the readable prose", () => {
  const { handler } = loadBroker();
  const r = invoke(handler, "docgen_workspace", {
    action: "render",
    format: "pdf",
    source: "nope.md",
  });
  assert.equal(r.ok, false);
  assert.match(r.output, /no such workspace file/);
});

test("mint: path → ref; refusal is readable", () => {
  const { handler, calls } = loadBroker({ stubs: { blobs: { "a.md": REF_SRC } } });
  const r = invoke(handler, "docgen_file_to_blob", { path: "a.md" });
  assert.equal(r.ok, true);
  assert.equal(r.output, REF_SRC);
  assert.deepEqual(calls.readBlob, ["a.md"]);

  const denied = invoke(handler, "docgen_file_to_blob", { path: "nope.bin" });
  assert.equal(denied.ok, false);
  assert.match(denied.output, /no such workspace file/);
});

test("materialize: ref → path; refusal is readable", () => {
  const { handler, calls } = loadBroker();
  const r = invoke(handler, "docgen_blob_to_file", { ref: ARTIFACT, path: "out/f.pdf" });
  assert.equal(r.ok, true);
  assert.equal(r.output, "wrote out/f.pdf");
  assert.deepEqual(norm(calls.writeBlob), [["out/f.pdf", ARTIFACT]]);

  const denied = invoke(handler, "docgen_blob_to_file", { ref: "", path: "x" });
  assert.equal(denied.ok, false);
  assert.match(denied.output, /pass `ref`/);
});

test("argument validation: action/format/assets shapes", () => {
  const { handler } = loadBroker();
  const base = { action: "render", format: "pdf", source: "a.md" };
  assert.match(invoke(handler, "docgen_workspace", { ...base, action: "paint" }).output, /render.*diagram/);
  assert.match(invoke(handler, "docgen_workspace", { ...base, format: "svg" }).output, /pdf, docx, pptx/);
  assert.match(invoke(handler, "docgen_workspace", { ...base, source: "" }).output, /pass `source`/);
  assert.match(invoke(handler, "docgen_workspace", { ...base, assets: "x" }).output, /\{name, path\}/);
  assert.match(
    invoke(handler, "docgen_workspace", { ...base, assets: [{}] }).output,
    /needs a path/,
  );
});
