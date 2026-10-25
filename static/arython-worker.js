// Web Worker that runs Arython code with the interpreter compiled to WebAssembly
// (GitHub Pages build, no server). Driven by browser-runtime.js; the Rust side of
// the imports below is compiler/src/host.rs, and the exports are in wasm.rs.

const EMIT_STDOUT = 0, EMIT_RESULT = 1, EMIT_ERROR = 2;
const INPUT_EOF = -1, INPUT_INTERRUPT = -2, INPUT_UNSUPPORTED = -3;

const encoder = new TextEncoder();
const decoder = new TextDecoder();

let wasm = null;
let inputCtrl = null;  // [state, length]: state 0 = waiting, 1 = answered, -1 = interrupted
let inputData = null;
let interruptFlag = null;  // [0] is set to 1 by the page to stop the running code

// The editor runs a whole program with its input typed in advance and collects the output
let script = null;  // { stdin: [lines], output, error }

const bytes = (ptr, len) => new Uint8Array(wasm.memory.buffer, ptr, len);
const readText = (ptr, len) => decoder.decode(bytes(ptr, len));

function emit(kind, ptr, len, line) {
    const text = readText(ptr, len);
    if (script) {
        if (kind === EMIT_STDOUT) script.output += text;
        else if (kind === EMIT_ERROR) script.error = { text, line: line || null };
    } else if (kind === EMIT_STDOUT) postMessage({ type: 'stream', name: 'stdout', text });
    else if (kind === EMIT_RESULT) postMessage({ type: 'result', text });
    else if (kind === EMIT_ERROR) postMessage({ type: 'error', text, line: line || null });
}

// Blocks until the page writes a reply into the shared buffer
function input(promptPtr, promptLen, outPtr, cap) {
    const prompt = readText(promptPtr, promptLen);
    let value;
    if (script) {
        // Like Python reading from a pipe: the prompt is printed, the answer isn't
        script.output += prompt;
        if (!script.stdin.length) return INPUT_EOF;
        value = script.stdin.shift();
    } else {
        if (!inputCtrl) return INPUT_UNSUPPORTED;
        postMessage({ type: 'input_request', prompt });
        Atomics.wait(inputCtrl, 0, 0);
        const state = inputCtrl[0];
        Atomics.store(inputCtrl, 0, 0);
        if (state < 0) return INPUT_INTERRUPT;
        // TextDecoder can't read shared memory directly, so copy first
        value = decoder.decode(inputData.slice(0, inputCtrl[1]));
        // Echo like a terminal would
        postMessage({ type: 'stream', name: 'stdout', text: `${prompt}${value}\n` });
    }
    const encoded = encoder.encode(value).subarray(0, cap);
    bytes(outPtr, encoded.length).set(encoded);
    return encoded.length;
}

const interrupted = () => (interruptFlag ? Atomics.exchange(interruptFlag, 0, 0) : 0);

// Returns 0 if interrupted
function sleepMs(ms) {
    if (interruptFlag) {
        Atomics.wait(interruptFlag, 0, 0, ms);
        return interrupted() ? 0 : 1;
    }
    const end = performance.now() + ms;
    while (performance.now() < end);
    return 1;
}

const host = { emit, input, now_ms: () => performance.timeOrigin + performance.now(), sleep_ms: sleepMs, interrupted };

async function init({ inputBuffer, interruptBuffer }) {
    if (inputBuffer) {
        inputCtrl = new Int32Array(inputBuffer, 0, 2);
        inputData = new Uint8Array(inputBuffer, 8);
    }
    if (interruptBuffer) interruptFlag = new Int32Array(interruptBuffer);
    const res = await fetch('arython.wasm');
    if (!res.ok) throw new Error(`arython.wasm: ${res.status}`);
    const { instance } = await WebAssembly.instantiate(await res.arrayBuffer(), { host });
    wasm = instance.exports;
    postMessage({ type: 'ready' });
}

// Copies code into the module's memory; the run_* export frees it
function call(fn, code) {
    const encoded = encoder.encode(code);
    const ptr = wasm.alloc(encoded.length);
    bytes(ptr, encoded.length).set(encoded);
    fn(ptr, encoded.length);
}

let loaded = null;
onmessage = async ({ data }) => {
    if (data.type === 'init') {
        loaded = init(data).catch(err => {
            postMessage({ type: 'load_error', text: String(err) });
            throw err;
        });
        return;
    }
    await loaded;
    try {
        run(data);
    } catch (err) {
        // A trap (the browser's stack ran out, or a bug in the interpreter) leaves the
        // module unusable, so the page replaces this worker
        const overflow = err instanceof RangeError || /too much recursion/i.test(String(err));
        postMessage({ type: 'crashed', overflow });
        close();
    }
};

function run(data) {
    if (data.type === 'cell') {
        call(wasm.run_cell, data.code);
        postMessage({ type: 'done' });
    } else if (data.type === 'script') {
        script = { stdin: data.stdin ? data.stdin.replace(/\r\n/g, '\n').replace(/\n$/, '').split('\n') : [], output: '', error: null };
        postMessage({ type: 'script_started' });
        try {
            call(wasm.run_script, data.code);
            postMessage({ type: 'script_result', output: script.output, error: script.error });
        } finally {
            script = null;
        }
    }
}
