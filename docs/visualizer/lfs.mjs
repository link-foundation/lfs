// A thin wrapper over the WebAssembly build of LFS (src/wasm.rs), shared by
// blueprint.html and the Node test (lfs.test.mjs). Build lfs.wasm with
// docs/visualizer/build-wasm.sh.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export class Lfs {
  /** @param {WebAssembly.Instance} instance */
  constructor(instance) {
    this.exports = instance.exports;
    this.exports.lfs_set_time(BigInt(Date.now()) * 1000000n);
  }

  /** Instantiates the module from its bytes (Node) or a URL (browser). */
  static async load(source = new URL("lfs.wasm", import.meta.url)) {
    let instance;
    if (source instanceof URL || typeof source === "string") {
      const response = await fetch(source);
      if (!response.ok) throw new Error(`lfs.wasm: ${response.status} ${response.statusText}`);
      ({ instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {}));
    } else {
      ({ instance } = await WebAssembly.instantiate(source, {}));
    }
    return new Lfs(instance);
  }

  output() {
    const { memory, lfs_output_ptr, lfs_output_len } = this.exports;
    return new Uint8Array(memory.buffer, lfs_output_ptr(), lfs_output_len()).slice();
  }

  /** Runs `call(ptr, len)` with `input` copied into module memory. */
  withInput(input, call) {
    const bytes = typeof input === "string" ? encoder.encode(input) : input;
    const ptr = this.exports.lfs_alloc(bytes.length);
    new Uint8Array(this.exports.memory.buffer, ptr, bytes.length).set(bytes);
    try {
      return call(ptr, bytes.length);
    } finally {
      this.exports.lfs_free(ptr, bytes.length);
    }
  }

  result(code) {
    const bytes = this.output();
    if (code !== 0) throw new Error(decoder.decode(bytes));
    return bytes;
  }

  json(code) {
    return JSON.parse(decoder.decode(this.result(code)));
  }

  /** `{demo, runtime, defaults}`: every configuration key with its help. */
  keys() {
    return this.json(this.exports.lfs_keys());
  }

  /** Builds a demo volume from `key=value` lines; returns its summary. */
  demo(params) {
    return this.json(this.withInput(params, (ptr, len) => this.exports.lfs_demo(ptr, len)));
  }

  /** Opens an image (Uint8Array); returns its summary. */
  open(image) {
    return this.json(this.withInput(image, (ptr, len) => this.exports.lfs_open(ptr, len)));
  }

  /** The space map (the `lfs map` JSON). */
  map() {
    return this.json(this.exports.lfs_map());
  }

  /** The blueprint node at `path` (array of child indices), `depth` levels deep. */
  node(path = [], depth = 1) {
    return this.json(this.withInput(path.join("/"), (ptr, len) => this.exports.lfs_node(ptr, len, depth)));
  }

  /** The path of the deepest node holding byte `offset`. */
  locate(offset) {
    return this.json(this.exports.lfs_locate(BigInt(offset)));
  }

  /** Raw image bytes. */
  bytes(offset, len) {
    return this.result(this.exports.lfs_bytes(BigInt(offset), BigInt(len)));
  }

  /** The whole image. */
  image() {
    return this.result(this.exports.lfs_image());
  }
}
