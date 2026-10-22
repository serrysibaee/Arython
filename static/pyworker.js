// Web Worker that runs Python with Pyodide (GitHub Pages build, no server).
// Driven by browser-python.js; the Python side lives in py/browser_runtime.py.
importScripts('https://cdn.jsdelivr.net/pyodide/v0.27.2/full/pyodide.js');

const PY_DIR = '/arython';
const PY_FILES = ['arabic_translator.py', 'kernel.py', 'browser_runtime.py'];

let runtime = null;
let inputCtrl = null;  // [state, length]: state 0 = waiting, 1 = answered, -1 = interrupted
let inputData = null;

const post = json => postMessage(JSON.parse(json));

// Called from Python: blocks until the page writes a reply into the shared buffer
function readInput(prompt) {
    if (!inputCtrl) return JSON.stringify({ unsupported: true });
    Atomics.wait(inputCtrl, 0, 0);
    const state = inputCtrl[0];
    Atomics.store(inputCtrl, 0, 0);
    if (state < 0) return JSON.stringify({ interrupt: true });
    // TextDecoder can't read shared memory directly, so copy first
    return JSON.stringify({ value: new TextDecoder().decode(inputData.slice(0, inputCtrl[1])) });
}

async function init({ inputBuffer, interruptBuffer }) {
    const pyodide = await loadPyodide();
    if (inputBuffer) {
        inputCtrl = new Int32Array(inputBuffer, 0, 2);
        inputData = new Uint8Array(inputBuffer, 8);
    }
    if (interruptBuffer) pyodide.setInterruptBuffer(new Uint8Array(interruptBuffer));
    pyodide.registerJsModule('arython_host', { send: post, read_input: readInput });

    pyodide.FS.mkdirTree(PY_DIR);
    for (const name of PY_FILES) {
        const res = await fetch(`../py/${name}`);
        if (!res.ok) throw new Error(`${name}: ${res.status}`);
        pyodide.FS.writeFile(`${PY_DIR}/${name}`, await res.text());
    }
    pyodide.runPython(`import sys; sys.path.insert(0, '${PY_DIR}')`);
    runtime = pyodide.pyimport('browser_runtime');
    postMessage({ type: 'ready' });
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
    if (data.type === 'cell') runtime.run_cell(data.code);
    else if (data.type === 'script') postMessage({ type: 'script_result', ...JSON.parse(runtime.run_script(data.code, data.stdin)) });
    else if (data.type === 'translate') postMessage({ type: 'translated', translated: JSON.parse(runtime.translate(JSON.stringify(data.codes))) });
};
