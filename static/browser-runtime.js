// Runs Arython in the browser for the GitHub Pages build: the interpreter (compiler/),
// compiled to WebAssembly, in a Web Worker (arython-worker.js).
// BrowserKernel stands in for the notebook's WebSocket and speaks the same messages
// as kernel_manager.py; ScriptRunner replaces the editor's /api/run.
import { translateToPython } from './translate.js';

const INTERRUPT_GRACE_MS = 2000;
const SCRIPT_TIMEOUT_MS = 5000;
const INPUT_BYTES = 64 * 1024;

// Live input and interrupts need shared memory, which needs cross-origin isolation
// (provided on GitHub Pages by coi-serviceworker.js)
const SHARED = typeof SharedArrayBuffer !== 'undefined' && self.crossOriginIsolated;

// Very deep recursion can exhaust the browser's own stack before Arython's limit of 1000 calls
const STACK_OVERFLOW = 'RecursionError: تجاوز عمق الاستدعاء ما يتحمله المتصفح';

class ArythonWorker {
    constructor(onMessage, onCrash) {
        this.inputBuffer = SHARED ? new SharedArrayBuffer(8 + INPUT_BYTES) : null;
        this.interruptBuffer = SHARED ? new SharedArrayBuffer(4) : null;
        this.worker = new Worker(new URL('arython-worker.js', import.meta.url));
        this.worker.onmessage = e => (e.data.type === 'crashed' ? onCrash(e.data.overflow) : onMessage(e.data));
        this.worker.onerror = e => { e.preventDefault(); onCrash(false); };
        this.worker.postMessage({ type: 'init', inputBuffer: this.inputBuffer, interruptBuffer: this.interruptBuffer });
    }

    post(msg) {
        if (this.interruptBuffer) Atomics.store(new Int32Array(this.interruptBuffer), 0, 0);
        this.worker.postMessage(msg);
    }

    reply(text) {
        const ctrl = new Int32Array(this.inputBuffer, 0, 2);
        // encodeInto can't write to shared memory, so encode then copy
        const bytes = new TextEncoder().encode(text).subarray(0, INPUT_BYTES);
        new Uint8Array(this.inputBuffer, 8).set(bytes);
        Atomics.store(ctrl, 1, bytes.length);
        Atomics.store(ctrl, 0, 1);
        Atomics.notify(ctrl, 0);
    }

    cancelInput() {
        const ctrl = new Int32Array(this.inputBuffer, 0, 2);
        Atomics.store(ctrl, 0, -1);
        Atomics.notify(ctrl, 0);
    }

    // Raises KeyboardInterrupt in the running code; false if the browser can't
    interrupt() {
        if (!this.interruptBuffer) return false;
        const flag = new Int32Array(this.interruptBuffer);
        Atomics.store(flag, 0, 1);
        Atomics.notify(flag, 0);  // wakes a time.sleep()
        return true;
    }

    terminate() { this.worker.terminate(); }
}

export class BrowserKernel {
    constructor() {
        this.readyState = 1;  // WebSocket.OPEN: cells can be queued while Python loads
        this.onmessage = null;
        this.onclose = null;
        this.pending = [];       // [cellId, code] waiting to run
        this.running = null;     // cell id currently executing
        this.waitingInput = false;
        this.interruptTimer = null;
        queueMicrotask(() => { this.emit({ type: 'kernel', state: 'loading' }); this.start(); });
    }

    emit(msg) { this.onmessage?.({ data: JSON.stringify(msg) }); }

    start() {
        this.ready = false;
        this.worker = new ArythonWorker(msg => this.fromWorker(msg), overflow => this.crash(overflow));
    }

    fromWorker(msg) {
        switch (msg.type) {
            case 'ready':
                this.ready = true;
                this.emit({ type: 'kernel', state: 'idle' });
                this.runNext();
                return;
            case 'load_error':
                this.readyState = 3;  // CLOSED
                this.worker.terminate();
                this.onclose?.();
                return;
            case 'input_request':
                this.waitingInput = true;
                break;
            case 'done':
                clearTimeout(this.interruptTimer);
                this.interruptTimer = null;
                break;
        }
        this.emit({ ...msg, cell_id: this.running });
        if (msg.type === 'done') {
            this.running = null;
            if (!this.pending.length) this.emit({ type: 'kernel', state: 'idle' });
            this.runNext();
        }
    }

    crash(overflow) {
        const text = overflow ? `${STACK_OVERFLOW}، فأُعيد تشغيل النواة (فُقدت المتغيرات)` : 'KernelDied';
        if (this.running) this.emit({ type: 'error', cell_id: this.running, text, line: null });
        this.restart('crash');
    }

    runNext() {
        if (!this.ready || this.running || !this.pending.length) return;
        const [cellId, code] = this.pending.shift();
        this.running = cellId;
        this.emit({ type: 'kernel', state: 'busy' });
        this.emit({ type: 'started', cell_id: cellId, translated: translateToPython(code) });
        this.worker.post({ type: 'cell', code });
    }

    restart(reason) {
        clearTimeout(this.interruptTimer);
        this.interruptTimer = null;
        this.worker.terminate();
        const cancelled = (this.running ? [this.running] : []).concat(this.pending.map(([id]) => id));
        this.pending = [];
        this.running = null;
        this.waitingInput = false;
        for (const id of cancelled) this.emit({ type: 'done', cell_id: id, cancelled: true });
        this.emit({ type: 'kernel', state: 'restarting', reason });
        this.start();
    }

    // Same messages the notebook sends over the WebSocket
    send(text) {
        const msg = JSON.parse(text);
        switch (msg.type) {
            case 'execute':
                this.pending.push([msg.cell_id, msg.code || '']);
                this.emit({ type: 'queued', cell_id: msg.cell_id });
                this.runNext();
                break;
            case 'input_reply':
                if (!this.waitingInput) break;
                this.waitingInput = false;
                this.worker.reply(msg.value || '');
                break;
            case 'interrupt': {
                const cancelled = this.pending.map(([id]) => id);
                this.pending = [];
                for (const id of cancelled) this.emit({ type: 'done', cell_id: id, cancelled: true });
                if (!this.running || this.interruptTimer) break;
                if (this.waitingInput) {
                    this.waitingInput = false;
                    this.worker.cancelInput();
                } else if (!this.worker.interrupt()) {
                    this.restart('interrupt_timeout');
                    break;
                }
                // If the cell doesn't stop in time, restart the kernel
                this.interruptTimer = setTimeout(() => this.restart('interrupt_timeout'), INTERRUPT_GRACE_MS);
                break;
            }
            case 'restart':
                this.restart('user');
                break;
        }
    }

    // Arabic → Python for the .ipynb export
    async translate(codes) {
        return codes.map(code => translateToPython(code));
    }
}

const TIMEOUT_MESSAGE = `خطأ: تجاوز البرنامج الحد الزمني (${SCRIPT_TIMEOUT_MS / 1000} ثوانٍ).`;

// The worker's { output, error: { text, line } | null } as /api/run reports it
function scriptResult({ output, error }) {
    if (!error) return { output, error: false };
    if (error.text === 'KeyboardInterrupt') return { output: TIMEOUT_MESSAGE, error: true };
    const where = error.line ? `\n(line ${error.line})` : '';
    return { output: `${output}خطأ في التنفيذ: ${error.text}${where}`, error: true };
}

export class ScriptRunner {
    constructor() { this.start(); }

    start() {
        this.ready = false;
        this.failed = false;
        this.worker = new ArythonWorker(msg => this.fromWorker(msg), overflow => this.finish({
            output: overflow ? `خطأ في التنفيذ: ${STACK_OVERFLOW}` : 'خطأ: توقف أرايثون بشكل مفاجئ.',
            error: true,
        }, true));
    }

    fromWorker(msg) {
        if (msg.type === 'ready') this.ready = true;
        else if (msg.type === 'load_error') {
            this.failed = true;  // retried on the next run
            this.finish({ output: `تعذّر تحميل أرايثون: ${msg.text}`, error: true });
        }
        else if (msg.type === 'script_started') {
            this.timer = setTimeout(() => this.timeout(), SCRIPT_TIMEOUT_MS);
        } else if (msg.type === 'script_result') this.finish(scriptResult(msg));
    }

    timeout() {
        if (this.worker.interrupt()) {
            this.timer = setTimeout(() => this.finish({ output: TIMEOUT_MESSAGE, error: true }, true), INTERRUPT_GRACE_MS);
        } else {
            this.finish({ output: TIMEOUT_MESSAGE, error: true }, true);
        }
    }

    finish(result, restart = false) {
        clearTimeout(this.timer);
        if (restart) { this.worker.terminate(); this.start(); }
        const resolve = this.resolve;
        this.resolve = null;
        resolve?.({ translated: this.translated || '', ...result });
    }

    // Resolves to { translated, output, error } like /api/run
    run(code, stdin) {
        if (this.failed) { this.worker.terminate(); this.start(); }
        this.translated = translateToPython(code);
        return new Promise(resolve => {
            this.resolve = resolve;
            this.worker.post({ type: 'script', code, stdin });
        });
    }
}
