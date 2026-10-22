// Runs Python in the browser (Pyodide in a Web Worker) for the GitHub Pages build.
// BrowserKernel stands in for the notebook's WebSocket and speaks the same messages
// as kernel_manager.py; ScriptRunner replaces the editor's /api/run.

const INTERRUPT_GRACE_MS = 2000;
const SCRIPT_TIMEOUT_MS = 5000;
const INPUT_BYTES = 64 * 1024;

// Live input and interrupts need shared memory, which needs cross-origin isolation
// (provided on GitHub Pages by coi-serviceworker.js)
const SHARED = typeof SharedArrayBuffer !== 'undefined' && self.crossOriginIsolated;

class PythonWorker {
    constructor(onMessage, onCrash) {
        this.inputBuffer = SHARED ? new SharedArrayBuffer(8 + INPUT_BYTES) : null;
        this.interruptBuffer = SHARED ? new SharedArrayBuffer(1) : null;
        this.worker = new Worker(new URL('pyworker.js', import.meta.url));
        this.worker.onmessage = e => onMessage(e.data);
        this.worker.onerror = e => { e.preventDefault(); onCrash(); };
        this.worker.postMessage({ type: 'init', inputBuffer: this.inputBuffer, interruptBuffer: this.interruptBuffer });
    }

    post(msg) {
        if (this.interruptBuffer) new Uint8Array(this.interruptBuffer)[0] = 0;
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
        new Uint8Array(this.interruptBuffer)[0] = 2;  // SIGINT
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
        this.translations = [];  // resolvers for translate() calls, in order
        queueMicrotask(() => { this.emit({ type: 'kernel', state: 'loading' }); this.start(); });
    }

    emit(msg) { this.onmessage?.({ data: JSON.stringify(msg) }); }

    start() {
        this.ready = false;
        this.worker = new PythonWorker(msg => this.fromWorker(msg), () => this.crash());
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
            case 'translated':
                this.translations.shift()?.resolve(msg.translated);
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

    crash() {
        if (this.running) this.emit({ type: 'error', cell_id: this.running, text: 'KernelDied', line: null });
        this.restart('crash');
    }

    runNext() {
        if (!this.ready || this.running || !this.pending.length) return;
        const [cellId, code] = this.pending.shift();
        this.running = cellId;
        this.emit({ type: 'kernel', state: 'busy' });
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
        for (const t of this.translations.splice(0)) t.reject(new Error('kernel restarted'));
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

    // Arabic → Python for the .ipynb export (waits for any running cell)
    translate(codes) {
        return new Promise((resolve, reject) => {
            this.translations.push({ resolve, reject });
            this.worker.post({ type: 'translate', codes });
        });
    }
}

const TIMEOUT_MESSAGE = `خطأ: تجاوز البرنامج الحد الزمني (${SCRIPT_TIMEOUT_MS / 1000} ثوانٍ).`;

export class ScriptRunner {
    constructor() { this.start(); }

    start() {
        this.ready = false;
        this.failed = false;
        this.worker = new PythonWorker(msg => this.fromWorker(msg), () => this.finish({ output: 'خطأ: توقف بايثون بشكل مفاجئ.', error: true }, true));
    }

    fromWorker(msg) {
        if (msg.type === 'ready') this.ready = true;
        else if (msg.type === 'load_error') {
            this.failed = true;  // retried on the next run
            this.finish({ output: `تعذّر تحميل بايثون: ${msg.text}`, error: true });
        }
        else if (msg.type === 'script_started') {
            this.translated = msg.translated;
            this.timer = setTimeout(() => this.timeout(), SCRIPT_TIMEOUT_MS);
        } else if (msg.type === 'script_result') this.finish(msg);
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
        this.translated = '';
        return new Promise(resolve => {
            this.resolve = resolve;
            this.worker.post({ type: 'script', code, stdin });
        });
    }
}
