const out = document.getElementById("output");

function log(...msg) {
  console.log(...msg);
  out.textContent += msg.join(" ") + "\n";
}

const memory = new WebAssembly.Memory({
  initial: 20,
});

async function load() {
  log("starting test...");

  const coreModule = await WebAssembly.instantiateStreaming(
    fetch("core.wasm"),
    { env: { memory } },
  );

  const decoderModule = await WebAssembly.instantiateStreaming(
    fetch("decoder.wasm"),
    { env: { memory } },
  );

  log("modules loaded");

  const core = coreModule.instance.exports;
  const decoder = decoderModule.instance.exports;

  core.fill_buffer();

  const ptr = core.buffer_ptr();
  const len = core.buffer_len();

  const view = new Uint8Array(memory.buffer, ptr, len);

  log("before:", JSON.stringify([...view]));

  decoder.process(ptr, len);

  log("after :", JSON.stringify([...view]));
}

load().catch((err) => {
  console.error(err);
  log("error:", err);
});
