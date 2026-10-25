// Arython notebook: code and text cells sharing one kernel over a WebSocket
// (or, in the GitHub Pages build, an in-browser kernel running the Arython interpreter).
import { createEditor, escapeHtml, toArabicDigits } from './editor.js';
import { BrowserKernel } from './browser-runtime.js';

const STORAGE_KEY = 'arython.notebook';
const MAX_OUTPUT_CHARS = 200_000;

const ERROR_NAMES = {
    SyntaxError: 'خطأ في الصياغة',
    IndentationError: 'خطأ في الإزاحة',
    NameError: 'اسم غير معرّف',
    TypeError: 'نوع غير مناسب',
    ValueError: 'قيمة غير صالحة',
    ZeroDivisionError: 'قسمة على صفر',
    IndexError: 'فهرس خارج النطاق',
    KeyError: 'مفتاح غير موجود',
    AttributeError: 'خاصية غير موجودة',
    ImportError: 'تعذّر الاستيراد',
    ModuleNotFoundError: 'وحدة غير موجودة',
    RecursionError: 'تكرار عودي زائد',
    EOFError: 'لا يوجد إدخال',
};

const WELCOME = [
    { type: 'text', source: '# مرحبا بك في دفتر أرايثون 👋\n\nهذا دفتر تفاعلي مثل **Colab**: كل خلية كود تعمل في نفس الجلسة، فالمتغيرات التي تعرّفها في خلية تبقى متاحة في الخلايا التالية.\n\n- شغّل خلية بالضغط على ▶ أو `Shift+Enter`\n- انقر مرتين على هذا النص لتحريره' },
    { type: 'code', source: 'الأسماء = ["سارة"، "خالد"، "ليلى"]\nاطبع("تم تعريف", طول(الأسماء)، "أسماء")' },
    { type: 'text', source: '## المتغيرات تبقى بين الخلايا\nالخلية التالية تستخدم `الأسماء` من الخلية السابقة. آخر سطر فيها تعبير، فتظهر قيمته تلقائيا.' },
    { type: 'code', source: 'لكل رقم، اسم في عدّد(الأسماء، ١):\n    اطبع(f"{رقم}. {اسم}")\n\nمرتب(الأسماء)' },
    { type: 'text', source: '## الإدخال التفاعلي\nعند استدعاء `أدخل()` يظهر مربع إدخال تحت الخلية.' },
    { type: 'code', source: 'الاسم = أدخل("ما اسمك؟ ")\nاطبع(f"أهلا يا {الاسم}!")' },
];

const cellsEl = document.getElementById('cells');
const titleEl = document.getElementById('nb-title');
const statusEl = document.getElementById('kernel-status');

let cells = [];          // { id, type, el, editor?, source, outputs, count, translated, state }
let activeId = null;
let execCounter = 0;
let nextId = 1;
let ws = null;
let kernelState = 'connecting';

// ---------- Markdown (small, safe subset) ----------
function renderMarkdown(src) {
    const blocks = [];
    // Pull out fenced code first so its contents aren't formatted
    src = src.replace(/```[^\n]*\n([\s\S]*?)```/g, (_, code) => {
        blocks.push(`<pre class="md-code">${escapeHtml(code.replace(/\n$/, ''))}</pre>`);
        return `\u0000${blocks.length - 1}\u0000`;
    });
    const inline = s => escapeHtml(s)
        .replace(/`([^`]+)`/g, '<code>$1</code>')
        .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
        .replace(/(^|[^*])\*([^*]+)\*/g, '$1<em>$2</em>')
        .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');

    const out = [];
    let list = null;
    let para = [];
    const flushPara = () => { if (para.length) out.push(`<p>${para.map(inline).join('<br>')}</p>`); para = []; };
    const flushList = () => { if (list) out.push(`<${list.tag}>${list.items.map(i => `<li>${inline(i)}</li>`).join('')}</${list.tag}>`); list = null; };

    for (const line of src.split('\n')) {
        let m;
        if ((m = line.match(/^\u0000(\d+)\u0000$/))) { flushPara(); flushList(); out.push(blocks[m[1]]); }
        else if ((m = line.match(/^(#{1,4})\s+(.*)$/))) { flushPara(); flushList(); out.push(`<h${m[1].length}>${inline(m[2])}</h${m[1].length}>`); }
        else if (/^(-{3,}|\*{3,})\s*$/.test(line)) { flushPara(); flushList(); out.push('<hr>'); }
        else if ((m = line.match(/^\s*[-*]\s+(.*)$/)) || (m = line.match(/^\s*[\d٠-٩]+[.)]\s+(.*)$/))) {
            flushPara();
            const tag = /^\s*[-*]/.test(line) ? 'ul' : 'ol';
            if (list && list.tag !== tag) flushList();
            list ??= { tag, items: [] };
            list.items.push(m[1]);
        }
        else if ((m = line.match(/^>\s?(.*)$/))) { flushPara(); flushList(); out.push(`<blockquote>${inline(m[1])}</blockquote>`); }
        else if (!line.trim()) { flushPara(); flushList(); }
        else { flushList(); para.push(line); }
    }
    flushPara(); flushList();
    return out.join('') || '<p class="md-empty">خلية نص فارغة — انقر مرتين للتحرير</p>';
}

// ---------- Cells ----------
function findCell(id) { return cells.find(c => c.id === id); }
function indexOf(id) { return cells.findIndex(c => c.id === id); }

function createCell(type, source = '', { outputs = [], count = null } = {}) {
    const cell = { id: nextId++, type, source, outputs, count, translated: '', state: 'idle' };
    const el = document.createElement('section');
    el.className = `cell cell-${type}`;
    el.dataset.id = cell.id;
    el.innerHTML = `
        <div class="cell-tools">
            ${type === 'code' ? '<button type="button" data-cell="translated" title="عرض كود بايثون">🐍</button>' : ''}
            <button type="button" data-cell="up" title="تحريك لأعلى">↑</button>
            <button type="button" data-cell="down" title="تحريك لأسفل">↓</button>
            <button type="button" data-cell="delete" title="حذف الخلية">🗑</button>
        </div>
        <div class="cell-main">
            ${type === 'code' ? `
            <div class="cell-gutter">
                <button type="button" class="run-btn" data-cell="run" title="تشغيل (Shift+Enter)">
                    <span class="play">▶</span><span class="spinner"></span>
                </button>
                <span class="exec-count"></span>
            </div>` : ''}
            <div class="cell-body">
                ${type === 'code' ? '<div class="cell-editor"></div>' : `
                <div class="md-view" dir="auto"></div>
                <textarea class="md-source" dir="auto" spellcheck="false" placeholder="اكتب نصا بتنسيق Markdown…"></textarea>`}
                <pre class="translated-view" dir="ltr" hidden></pre>
                <div class="cell-output" dir="auto"></div>
            </div>
        </div>
        <div class="insert-bar">
            <button type="button" data-cell="insert-code">+ كود</button>
            <button type="button" data-cell="insert-text">+ نص</button>
        </div>`;
    cell.el = el;

    if (type === 'code') {
        cell.editor = createEditor(el.querySelector('.cell-editor'), {
            value: source,
            autoHeight: true,
            onRun: mode => runCell(cell.id, mode),
            onChange: () => { cell.source = cell.editor.value; save(); },
            onFocus: () => setActive(cell.id),
        });
    } else {
        const ta = el.querySelector('.md-source');
        ta.value = source;
        ta.addEventListener('input', () => { cell.source = ta.value; autoGrow(ta); save(); });
        ta.addEventListener('focus', () => setActive(cell.id));
        ta.addEventListener('keydown', e => {
            if (e.key === 'Enter' && (e.shiftKey || e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                finishText(cell.id, e.shiftKey ? 'next' : 'stay');
            } else if (e.key === 'Escape') {
                finishText(cell.id, 'stay');
            }
        });
        el.querySelector('.md-view').addEventListener('dblclick', () => editText(cell.id));
    }
    el.addEventListener('mousedown', () => setActive(cell.id));
    renderCell(cell);
    return cell;
}

function autoGrow(ta) {
    ta.style.height = 'auto';
    ta.style.height = `${ta.scrollHeight + 2}px`;
}

function insertCell(type, index, source = '', focus = true) {
    const cell = createCell(type, source);
    cells.splice(index, 0, cell);
    const after = cells[index + 1];
    cellsEl.insertBefore(cell.el, after ? after.el : null);
    if (type === 'text' && !source) editText(cell.id);
    else if (focus) focusCell(cell.id);
    save();
    return cell;
}

function focusCell(id) {
    const cell = findCell(id);
    if (!cell) return;
    setActive(id);
    if (cell.type === 'code') cell.editor.focus(true);
    else cell.el.scrollIntoView({ block: 'nearest' });
}

function setActive(id) {
    if (activeId === id) return;
    findCell(activeId)?.el.classList.remove('active');
    activeId = id;
    findCell(id)?.el.classList.add('active');
}

function editText(id) {
    const cell = findCell(id);
    cell.el.classList.add('editing');
    const ta = cell.el.querySelector('.md-source');
    autoGrow(ta);
    ta.focus();
}

function finishText(id, mode) {
    const cell = findCell(id);
    cell.el.classList.remove('editing');
    renderCell(cell);
    if (mode === 'next') advanceFrom(id);
}

function advanceFrom(id) {
    const i = indexOf(id);
    if (i === cells.length - 1) insertCell('code', i + 1);
    else focusCell(cells[i + 1].id);
}

function deleteCell(id) {
    const i = indexOf(id);
    if (i < 0) return;
    cells[i].el.remove();
    cells.splice(i, 1);
    if (!cells.length) insertCell('code', 0);
    else focusCell(cells[Math.min(i, cells.length - 1)].id);
    save();
}

function moveCell(id, delta) {
    const i = indexOf(id);
    const j = i + delta;
    if (j < 0 || j >= cells.length) return;
    [cells[i], cells[j]] = [cells[j], cells[i]];
    const [first, second] = i < j ? [cells[i], cells[j]] : [cells[j], cells[i]];
    cellsEl.insertBefore(first.el, second.el);
    save();
}

// ---------- Rendering ----------
function renderCell(cell) {
    if (cell.type === 'text') {
        cell.el.querySelector('.md-view').innerHTML = renderMarkdown(cell.source);
        return;
    }
    const count = cell.state === 'queued' ? '*' : cell.count != null ? toArabicDigits(cell.count) : ' ';
    cell.el.querySelector('.exec-count').textContent = `[${count}]`;
    cell.el.classList.toggle('running', cell.state === 'running');
    cell.el.classList.toggle('queued', cell.state === 'queued');
    cell.el.querySelector('.translated-view').textContent = cell.translated;
    renderOutputs(cell);
}

function renderOutputs(cell) {
    const out = cell.el.querySelector('.cell-output');
    out.innerHTML = '';
    for (const o of cell.outputs) out.appendChild(outputNode(o));
    out.classList.toggle('empty', !cell.outputs.length);
}

function outputNode(o) {
    const pre = document.createElement('pre');
    pre.dir = 'auto';
    if (o.kind === 'stream') { pre.className = `out-stream out-${o.name}`; pre.textContent = o.text; }
    else if (o.kind === 'result') { pre.className = 'out-result'; pre.dir = 'ltr'; pre.textContent = o.text; }
    else if (o.kind === 'error') { pre.className = 'out-error'; pre.textContent = o.text; }
    else if (o.kind === 'info') { pre.className = 'out-info'; pre.textContent = o.text; }
    return pre;
}

function outputLength(cell) {
    return cell.outputs.reduce((n, o) => n + o.text.length, 0);
}

function appendStream(cell, name, text) {
    if (outputLength(cell) > MAX_OUTPUT_CHARS) {
        if (!cell.truncated) {
            cell.truncated = true;
            addOutput(cell, { kind: 'info', text: '… تم اقتطاع الناتج لأنه طويل جدا' });
        }
        return;
    }
    const last = cell.outputs[cell.outputs.length - 1];
    const out = cell.el.querySelector('.cell-output');
    if (last && last.kind === 'stream' && last.name === name) {
        last.text += text;
        out.lastChild.textContent = last.text;
    } else {
        addOutput(cell, { kind: 'stream', name, text });
    }
}

function addOutput(cell, o) {
    cell.outputs.push(o);
    const out = cell.el.querySelector('.cell-output');
    out.classList.remove('empty');
    out.appendChild(outputNode(o));
}

function formatError(text, line) {
    if (text === 'KernelDied') return '⚠ توقفت النواة بشكل مفاجئ وأُعيد تشغيلها (فُقدت المتغيرات)';
    if (text.startsWith('KeyboardInterrupt')) return '■ تم إيقاف التنفيذ';
    const [, name, msg] = text.match(/^(\w+): ?([\s\S]*)$/) || [null, text, ''];
    const arabic = ERROR_NAMES[name];
    let s = `⚠ ${arabic ? `${arabic} (${name})` : name}`;
    if (msg) s += `: ${msg}`;
    if (line) s += `\nفي السطر ${toArabicDigits(line)}`;
    return s;
}

function showInputPrompt(cell, prompt) {
    const box = document.createElement('form');
    box.className = 'out-input';
    box.innerHTML = `<label></label><input type="text" dir="auto" autocomplete="off"><button type="submit">إدخال ↵</button>`;
    box.querySelector('label').textContent = prompt || 'إدخال:';
    const input = box.querySelector('input');
    box.addEventListener('submit', e => {
        e.preventDefault();
        box.remove();
        send({ type: 'input_reply', value: input.value });
    });
    cell.el.querySelector('.cell-output').appendChild(box);
    cell.el.querySelector('.cell-output').classList.remove('empty');
    input.focus();
}

// ---------- Running ----------
function runCell(id, mode = 'stay') {
    const cell = findCell(id);
    if (cell.type === 'text') { finishText(id, mode); return; }
    if (!ws || ws.readyState !== WebSocket.OPEN) {
        cell.outputs = [{ kind: 'error', text: '⚠ غير متصل بالخادم' }];
        renderOutputs(cell);
        return;
    }
    cell.outputs = [];
    cell.truncated = false;
    cell.state = 'queued';
    cell.editor.setErrorLine(null);
    renderCell(cell);
    send({ type: 'execute', cell_id: cell.id, code: cell.editor.value });

    if (mode === 'next') advanceFrom(id);
    else if (mode === 'insert') insertCell('code', indexOf(id) + 1);
}

function runAll() {
    for (const cell of cells) {
        if (cell.type === 'code' && cell.editor.value.trim()) runCell(cell.id);
    }
}

function clearOutputs() {
    for (const cell of cells) {
        if (cell.type !== 'code') continue;
        cell.outputs = [];
        cell.count = null;
        cell.editor.setErrorLine(null);
        renderCell(cell);
    }
    save();
}

function handleMessage(msg) {
    if (msg.type === 'kernel') { setKernelState(msg.state, msg.reason); return; }
    const cell = findCell(msg.cell_id);
    if (!cell) return;

    switch (msg.type) {
        case 'queued':
            cell.state = 'queued';
            break;
        case 'started':
            cell.state = 'running';
            cell.count = ++execCounter;
            cell.translated = msg.translated;
            break;
        case 'stream':
            appendStream(cell, msg.name, msg.text);
            return;
        case 'result':
            addOutput(cell, { kind: 'result', text: msg.text });
            return;
        case 'error':
            addOutput(cell, { kind: 'error', text: formatError(msg.text, msg.line) });
            if (msg.line) cell.editor.setErrorLine(msg.line);
            return;
        case 'input_request':
            showInputPrompt(cell, msg.prompt);
            return;
        case 'done':
            cell.state = 'idle';
            cell.el.querySelector('.out-input')?.remove();
            save();
            break;
    }
    renderCell(cell);
}

// ---------- Kernel connection ----------
const KERNEL_LABELS = {
    connecting: 'جارٍ الاتصال…',
    loading: 'جارٍ تحميل أرايثون…',
    idle: 'النواة جاهزة',
    busy: 'النواة مشغولة',
    restarting: 'إعادة تشغيل النواة…',
    disconnected: 'غير متصل — انقر لإعادة الاتصال',
};

function setKernelState(state, reason) {
    kernelState = state;
    statusEl.dataset.state = state;
    statusEl.querySelector('.label').textContent = KERNEL_LABELS[state] || state;
    if (state === 'restarting') {
        execCounter = 0;
        if (reason === 'interrupt_timeout') toast('لم يتوقف التنفيذ، فأُعيد تشغيل النواة');
    }
}

function send(msg) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function connect() {
    setKernelState('connecting');
    if (window.ARYTHON_MODE === 'browser') {
        ws = new BrowserKernel();
    } else {
        const proto = location.protocol === 'https:' ? 'wss' : 'ws';
        ws = new WebSocket(`${proto}://${location.host}/ws/notebook`);
    }
    ws.onmessage = e => handleMessage(JSON.parse(e.data));
    ws.onclose = () => {
        setKernelState('disconnected');
        execCounter = 0;
        for (const cell of cells) {
            if (cell.type === 'code' && cell.state !== 'idle') { cell.state = 'idle'; renderCell(cell); }
        }
    };
}
statusEl.addEventListener('click', () => { if (kernelState === 'disconnected') connect(); });

function toast(text) {
    const t = document.createElement('div');
    t.className = 'toast';
    t.textContent = text;
    document.body.appendChild(t);
    setTimeout(() => t.remove(), 3500);
}

// ---------- Save / load ----------
function serialize() {
    return {
        format: 'arython-notebook',
        version: 1,
        title: titleEl.value,
        cells: cells.map(c => ({
            type: c.type,
            source: c.type === 'code' ? c.editor.value : c.source,
            ...(c.type === 'code' ? { outputs: c.outputs, count: c.count } : {}),
        })),
    };
}

let saveTimer = null;
function save() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
        try { localStorage.setItem(STORAGE_KEY, JSON.stringify(serialize())); } catch { /* storage unavailable */ }
    }, 300);
}

function load(data) {
    cellsEl.innerHTML = '';
    cells = [];
    activeId = null;
    titleEl.value = data.title || 'دفتر بلا عنوان';
    for (const c of data.cells || []) {
        const cell = createCell(c.type === 'text' ? 'text' : 'code', c.source || '', { outputs: c.outputs || [], count: c.count ?? null });
        cells.push(cell);
        cellsEl.appendChild(cell.el);
    }
    if (!cells.length) insertCell('code', 0, '', false);
    save();
}

function download(name, content, type) {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([content], { type }));
    a.download = name;
    a.click();
    URL.revokeObjectURL(a.href);
}

const safeName = () => (titleEl.value.trim() || 'دفتر').replace(/[\\/:*?"<>|]/g, '_');

async function exportIpynb() {
    const codes = cells.filter(c => c.type === 'code').map(c => c.editor.value);
    const translated = ws instanceof BrowserKernel ? await ws.translate(codes) : (await (await fetch('/api/translate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ codes }),
    })).json()).translated;
    let k = 0;
    const toLines = s => s.split('\n').map((l, i, a) => i < a.length - 1 ? l + '\n' : l);
    const nb = {
        nbformat: 4,
        nbformat_minor: 5,
        metadata: { kernelspec: { name: 'python3', display_name: 'Python 3', language: 'python' } },
        cells: cells.map(c => c.type === 'text'
            ? { cell_type: 'markdown', metadata: {}, source: toLines(c.source) }
            : { cell_type: 'code', metadata: {}, execution_count: null, outputs: [], source: toLines(translated[k++]) }),
    };
    download(`${safeName()}.ipynb`, JSON.stringify(nb, null, 1), 'application/json');
}

// ---------- Toolbar & cell buttons ----------
const actions = {
    'add-code': () => insertCell('code', activeId ? indexOf(activeId) + 1 : cells.length),
    'add-text': () => insertCell('text', activeId ? indexOf(activeId) + 1 : cells.length),
    'run-all': runAll,
    'interrupt': () => send({ type: 'interrupt' }),
    'restart': () => { if (confirm('إعادة تشغيل النواة؟ ستُحذف كل المتغيرات.')) send({ type: 'restart' }); },
    'clear-outputs': clearOutputs,
    'new': () => { if (confirm('إنشاء دفتر جديد؟ سيُستبدل الدفتر الحالي.')) { load({ cells: [{ type: 'code', source: '' }] }); send({ type: 'restart' }); } },
    'open': () => document.getElementById('open-file').click(),
    'save': () => download(`${safeName()}.arnb`, JSON.stringify(serialize(), null, 1), 'application/json'),
    'export': exportIpynb,
};

document.querySelector('.nb-toolbar').addEventListener('click', e => {
    const action = e.target.closest('[data-action]')?.dataset.action;
    if (action) actions[action]();
});

cellsEl.addEventListener('click', e => {
    const btn = e.target.closest('[data-cell]');
    if (!btn) return;
    const id = Number(btn.closest('.cell').dataset.id);
    const cell = findCell(id);
    switch (btn.dataset.cell) {
        case 'run': runCell(id); break;
        case 'up': moveCell(id, -1); break;
        case 'down': moveCell(id, 1); break;
        case 'delete': deleteCell(id); break;
        case 'insert-code': insertCell('code', indexOf(id) + 1); break;
        case 'insert-text': insertCell('text', indexOf(id) + 1); break;
        case 'translated': {
            const view = cell.el.querySelector('.translated-view');
            view.hidden = !view.hidden;
            if (!view.textContent) view.textContent = '# شغّل الخلية لرؤية كود بايثون';
            break;
        }
    }
});

document.getElementById('open-file').addEventListener('change', async e => {
    const file = e.target.files[0];
    e.target.value = '';
    if (!file) return;
    try {
        const data = JSON.parse(await file.text());
        if (data.format !== 'arython-notebook') throw new Error();
        load(data);
        send({ type: 'restart' });
    } catch {
        toast('الملف ليس دفتر أرايثون صالحا');
    }
});

titleEl.addEventListener('input', save);

// ---------- Start ----------
let saved = null;
try { saved = JSON.parse(localStorage.getItem(STORAGE_KEY)); } catch { /* ignore */ }
load(saved?.cells?.length ? saved : { title: 'مرحبا بأرايثون', cells: WELCOME });
connect();
