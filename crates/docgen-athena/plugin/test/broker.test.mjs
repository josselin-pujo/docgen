// Node smoke tests for the docgen broker. The host-injected `Athena`
// global is faked (per the skill's testing note); the broker source is
// loaded as a script with the stub in scope, and its handler is driven
// directly. Run: `node --test crates/docgen-athena/plugin/test/`.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const source = readFileSync(path.join(here, "..", "broker.js"), "utf8");

// Cross-realm-safe plain-data comparison.
const norm = (x) => JSON.parse(JSON.stringify(x));

function loadBroker(opts = {}) {
  const calls = { readBlob: [], read: [], writeBlob: [], put: [], invoke: [], toolInvoke: [] };
  const stubs = opts.stubs || {};

  globalThis.Athena = {
    serve(h) {
      loadBroker.handler = h;
    },
    fs: {
      readBlob(p) {
        calls.readBlob.push(p);
        if (stubs.readBlob) return stubs.readBlob(p);
        return "sha256:file";
      },
      read(p) {
        calls.read.push(p);
        if (stubs.read) return stubs.read(p);
        throw new Error("no fs.read stub");
      },
      writeBlob(p, d) {
        calls.writeBlob.push([p, d]);
        if (stubs.writeBlob) return stubs.writeBlob(p, d);
      },
    },
    blob: {
      put(b, ttl) {
        calls.put.push(b);
        if (stubs.blobPut) return stubs.blobPut(b, ttl);
        return "sha256:inline";
      },
    },
    wasm: {
      invoke(tool, input) {
        calls.invoke.push([tool, input]);
        if (stubs.wasmInvoke) return stubs.wasmInvoke(tool, input);
        return { ok: { artifact: { ref: "sha256:artifact" } } };
      },
    },
    tool: {
      invoke(key, tool, args) {
        calls.toolInvoke.push([key, tool, args]);
        if (stubs.toolInvoke) return stubs.toolInvoke(key, tool, args);
        return { ok: true, output: "ok" };
      },
    },
    caps: { list: () => [] },
  };
  globalThis.__athenaManifest = opts.manifest || { config: {} };

  // eslint-disable-next-line no-new-func
  new Function(source)();
  return { handler: loadBroker.handler, calls };
}

const invoke = (handler, args, callKey = "c1") =>
  handler({ op: "invoke", tool: "docgen_compose", callKey, args });

test("specs: exactly one docgen_compose ToolSpec with a JSON-Schema object", () => {
  const { handler } = loadBroker();
  const r = handler({ op: "specs" });
  assert.equal(r.specs.length, 1);
  const s = r.specs[0];
  assert.equal(s.name, "docgen_compose");
  assert.equal(typeof s.description, "string");
  assert.equal(s.parameters.type, "object");
  assert.deepEqual(s.parameters.required, ["action", "format"]);
  assert.ok(s.parameters.properties.action && s.parameters.properties.format);
});

test("sections: empty list", () => {
  const { handler } = loadBroker();
  assert.deepEqual(handler({ op: "sections" }), { sections: [] });
});

test("unknown ops answer honestly", () => {
  const { handler } = loadBroker();
  const r = handler({ op: "wat" });
  assert.equal(r.ok, false);
  assert.equal(r.output, "unknown op: wat");
});

test("render from a workspace file: readBlob → wasm → writeBlob → surface", () => {
  const { handler, calls } = loadBroker();
  const r = invoke(handler, { action: "render", format: "pdf", source: "docs/doc.md" });
  assert.equal(r.ok, true);
  assert.match(r.output, /^wrote docs\/doc\.pdf \(pdf\)/);
  assert.match(r.output, /sha256:artifact/);

  assert.deepEqual(calls.readBlob, ["docs/doc.md"]);
  const [tool, input] = calls.invoke[0];
  assert.equal(tool, "docgen_compose");
  assert.deepEqual(norm(input), {
    action: "render",
    format: "pdf",
    kind: "",
    content: { ref: "sha256:file" },
  });
  assert.deepEqual(norm(calls.writeBlob), [["docs/doc.pdf", "sha256:artifact"]]);
  assert.deepEqual(norm(calls.toolInvoke[0]), [
    "c1:surface",
    "load_artifact",
    { digest: "sha256:artifact" },
  ]);
});

test("inline content: blob.put once, default out name", () => {
  const { handler, calls } = loadBroker();
  const r = invoke(handler, { action: "render", format: "docx", content: "# Hi" });
  assert.equal(r.ok, true);
  assert.match(r.output, /^wrote document\.docx \(docx\)/);
  assert.equal(calls.put.length, 1);
  assert.equal(Buffer.from(calls.put[0]).toString(), "# Hi");
  assert.deepEqual(norm(calls.invoke[0][1].content), { ref: "sha256:inline" });
  assert.deepEqual(calls.readBlob, []);
});

test("diagram kind inference: mermaid text, excalidraw JSON, extension, explicit wins", () => {
  const m1 = loadBroker();
  const r1 = invoke(m1.handler, { action: "diagram", format: "svg", content: "flowchart LR\nA-->B" });
  assert.equal(r1.ok, true);
  assert.equal(m1.calls.invoke[0][1].kind, "mermaid");
  assert.match(r1.output, /^wrote diagram\.svg/);

  const m2 = loadBroker();
  const r2 = invoke(m2.handler, {
    action: "diagram",
    format: "png",
    content: '{"type":"excalidraw","elements":[]}',
  });
  assert.equal(r2.ok, true);
  assert.equal(m2.calls.invoke[0][1].kind, "excalidraw");

  const m3 = loadBroker();
  invoke(m3.handler, { action: "diagram", format: "svg", source: "scene.excalidraw" });
  assert.equal(m3.calls.invoke[0][1].kind, "excalidraw");

  const m4 = loadBroker({
    stubs: { read: () => new TextEncoder().encode("flowchart TD\nX-->Y") },
  });
  invoke(m4.handler, { action: "diagram", format: "svg", source: "weird.bin" });
  assert.equal(m4.calls.invoke[0][1].kind, "mermaid");

  const m5 = loadBroker();
  invoke(m5.handler, {
    action: "diagram",
    format: "svg",
    content: "flowchart LR",
    kind: "excalidraw",
  });
  assert.equal(m5.calls.invoke[0][1].kind, "excalidraw");
});

test("argument refusals are honest prose", () => {
  const { handler } = loadBroker();
  assert.match(invoke(handler, { action: "render", format: "pdf" }).output, /exactly one of/);
  assert.match(
    invoke(handler, { action: "render", format: "pdf", source: "a.md", content: "x" }).output,
    /not both/,
  );
  assert.match(invoke(handler, { action: "paint", format: "pdf", content: "x" }).output, /render.*diagram/);
  assert.match(invoke(handler, { action: "render", format: "svg", content: "x" }).output, /pdf, docx, pptx/);
  const { handler: h2 } = loadBroker();
  assert.match(invoke(h2, { action: "diagram", format: "svg", source: "x.bin" }).output, /kind/);
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

test("writeBlob refusal keeps the digest and fails honestly", () => {
  const { handler } = loadBroker({
    stubs: {
      writeBlob: () => {
        throw new Error("fs.write refused: missing grant fs.write=docgen");
      },
    },
  });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, false);
  assert.match(r.output, /could not write document\.pdf/);
  assert.match(r.output, /fs\.write refused/);
  assert.match(r.output, /sha256:artifact/);
});

test("surfacing is best-effort: its failure never fails the tool", () => {
  const { handler } = loadBroker({
    stubs: {
      toolInvoke: () => {
        throw new Error("load_artifact unavailable");
      },
    },
  });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, true);
});

test("source read refusal fails with the readable prose", () => {
  const { handler } = loadBroker({
    stubs: {
      readBlob: () => {
        throw new Error('no such workspace file: "nope.md"');
      },
    },
  });
  const r = invoke(handler, { action: "render", format: "pdf", source: "nope.md" });
  assert.equal(r.ok, false);
  assert.match(r.output, /no such workspace file/);
});

test("config: surface=false skips surfacing, debug is tolerated", () => {
  const { handler, calls } = loadBroker({
    manifest: { config: { surface: false, debug: true } },
  });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, true);
  assert.equal(calls.toolInvoke.length, 0);
});

test("config may arrive as a JSON string (parse-once discipline)", () => {
  const { handler, calls } = loadBroker({ manifest: { config: '{"surface":false}' } });
  const r = invoke(handler, { action: "render", format: "pdf", content: "x" });
  assert.equal(r.ok, true);
  assert.equal(calls.toolInvoke.length, 0);
});
