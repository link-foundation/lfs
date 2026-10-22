// Tests the WebAssembly build the blueprint page uses:
//   bash docs/visualizer/build-wasm.sh && node --test docs/visualizer/lfs.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { Lfs } from "./lfs.mjs";

const wasm = readFileSync(new URL("lfs.wasm", import.meta.url));
const lfs = await Lfs.load(wasm);

// The configurations most likely to be chosen, plus the extremes.
const CONFIGS = [
  "",
  "layout=split; index=adaptive",
  "layout=data-only; index=bitstring",
  "layout=united; index=tree; journal=full; names=packed",
  "journal-blocks=0; journal=off; compact=true",
  "block-size=4096; size=8M",
  "block-size=65536; size=8M; files=4",
];

/** Checks that the children of `node` cover it exactly, recursively. */
function assertPartition(node) {
  if (!node.children || node.children.length === 0) return;
  let next = node.offset;
  for (const child of node.children) {
    assert.equal(child.offset, next, `gap before ${child.name} in ${node.name}`);
    next += child.size;
    assertPartition(child);
  }
  assert.equal(next, node.offset + node.size, `${node.name} is not covered`);
}

test("every configuration key is listed with its help and default", () => {
  const keys = lfs.keys();
  const all = [...keys.demo, ...keys.runtime].map(([key]) => key);
  for (const key of ["layout", "block-size", "journal-blocks", "names", "index", "bitstring-threshold", "journal", "gap-fill"]) {
    assert.ok(all.includes(key), key);
  }
  for (const [key, help] of [...keys.demo, ...keys.runtime]) assert.ok(help.length > 0, key);
  for (const key of all) assert.match(keys.defaults, new RegExp(`^${key}=`, "m"), key);
});

test("demo volumes of every configuration partition every byte", () => {
  for (const config of CONFIGS) {
    const summary = lfs.demo(`size=1M; block-size=1024; files=8; file-size=3000; ${config}`);
    const root = lfs.node([], 3);
    assert.equal(root.offset, 0);
    assert.equal(root.size, summary.image_len, config);
    assertPartition(root);
    const map = lfs.map();
    assert.equal(map.format, "lfs-map");
    assert.equal(map.total_blocks * map.block_size, summary.image_len);
  }
});

test("nodes expand by path down to single bytes", () => {
  lfs.demo("size=1M; block-size=1024; layout=united; index=adaptive");
  const superblock = lfs.node([0], 1);
  assert.equal(superblock.name, "superblock");
  const magic = superblock.children.find((child) => child.name === "magic");
  assert.equal(magic.value, "LINKSFS\\x00");
  const layout = superblock.children.find((child) => child.name === "layout");
  assert.equal(layout.size, 1);
  assert.equal(layout.value, "0");
  assert.equal(layout.note, "united");
  // Every byte locates a leaf whose range holds it.
  for (const offset of [0, 40, 1024, 5000, 200000, 1048575]) {
    const path = lfs.locate(offset);
    const node = lfs.node(path, 0);
    assert.ok(node.offset <= offset && offset < node.offset + node.size, `${offset} in ${node.name}`);
  }
});

test("raw bytes match the image and a downloaded image reopens", () => {
  const summary = lfs.demo("size=512K; block-size=1024; files=3; label=round-trip");
  const image = lfs.image();
  assert.equal(image.length, summary.image_len);
  assert.deepEqual(lfs.bytes(0, 8), image.slice(0, 8));
  assert.equal(new TextDecoder().decode(image.slice(0, 7)), "LINKSFS");
  const reopened = lfs.open(image);
  assert.equal(reopened.label, "round-trip");
  assert.equal(reopened.files, summary.files);
  assert.deepEqual(lfs.node([], 2), (lfs.open(image), lfs.node([], 2)));
});

test("errors come back as exceptions", () => {
  assert.throws(() => lfs.demo("colour=blue"), /unknown demo key/);
  assert.throws(() => lfs.open(new Uint8Array(4096)), /magic/);
  assert.throws(() => lfs.node([99], 1), /./);
});
