// Arabic code editor: a transparent <textarea> over a highlighted <pre>,
// with line numbers, indent guides, and editing helpers.
// Shared by the single-file page and the notebook.

const KEYWORDS = window.ARYTHON_KEYWORDS || {};
const BUILTINS = new Set([
    'print', 'input', 'len', 'type', 'range', 'int', 'float', 'str', 'list', 'sum', 'max', 'min',
    'abs', 'round', 'sorted', 'dict', 'enumerate',
    'append', 'items', 'keys', 'values', 'split', 'join', 'upper',
]);
const CONSTANTS = new Set(['True', 'False', 'None']);
const INDENT = '    ';
const PAIRS = { '(': ')', '[': ']', '{': '}', '"': '"', "'": "'" };

export const escapeHtml = s => s.replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
export const toArabicDigits = n => String(n).replace(/\d/g, d => '٠١٢٣٤٥٦٧٨٩'[d]);

const span = (cls, text) => `<span class="tok-${cls}">${escapeHtml(text)}</span>`;
const isIdent = ch => /[\p{L}\p{M}\p{N}_ـ]/u.test(ch);
const isDigit = ch => /[0-9٠-٩۰-۹]/.test(ch);

function classify(word) {
    const py = KEYWORDS[word];
    if (py === undefined) return null;
    if (CONSTANTS.has(py)) return 'const';
    if (BUILTINS.has(py)) return 'builtin';
    if (/^[=!<>]/.test(py)) return 'op';
    return 'kw';
}

export function highlightLine(line) {
    let html = '';
    let i = 0;

    // Indentation guides: one dotted marker per indent level
    const lead = line.match(/^ */)[0].length;
    for (; i + 4 <= lead; i += 4) html += '<span class="indent">    </span>';
    if (i < lead) { html += line.slice(i, lead); i = lead; }

    let expectName = false;
    while (i < line.length) {
        const ch = line[i];
        if (ch === '#') { html += span('comment', line.slice(i)); break; }
        if (ch === '"' || ch === "'") {
            let j = i + 1;
            while (j < line.length && line[j] !== ch) j += line[j] === '\\' ? 2 : 1;
            html += span('string', line.slice(i, j + 1));
            i = j + 1;
            continue;
        }
        if (isDigit(ch)) {
            let j = i;
            while (j < line.length && (isDigit(line[j]) || line[j] === '.' || line[j] === '٫')) j++;
            html += span('number', line.slice(i, j));
            i = j;
            continue;
        }
        if (isIdent(ch)) {
            let j = i;
            while (j < line.length && isIdent(line[j])) j++;
            const word = line.slice(i, j);
            const cls = classify(word);
            if (expectName) { html += span('defname', word); expectName = false; }
            else if (cls) { html += span(cls, word); expectName = ['def', 'class'].includes(KEYWORDS[word]); }
            else if (line[j] === '(') html += span('call', word);
            else html += escapeHtml(word);
            i = j;
            continue;
        }
        if ('()[]{}'.includes(ch)) html += span('bracket', ch);
        else if ('+-*/%=<>!:,،؛.'.includes(ch)) html += span('punct', ch);
        else html += escapeHtml(ch);
        i++;
    }
    return html;
}

/**
 * Create an editor inside `root`.
 * options:
 *   value        initial code
 *   placeholder  textarea placeholder
 *   autoHeight   grow with content instead of scrolling (notebook cells)
 *   onRun(mode)  called with 'stay' (Ctrl+Enter), 'next' (Shift+Enter), 'insert' (Alt+Enter)
 *   onChange()   called after every edit
 *   onCursor(line, col)  called when the caret moves
 *   onFocus()    called when the editor gains focus
 */
export function createEditor(root, options = {}) {
    root.classList.add('editor');
    if (options.autoHeight) root.classList.add('auto-height');
    root.innerHTML = `
        <div class="gutter" aria-hidden="true"></div>
        <div class="code-wrap">
            <pre class="highlight" aria-hidden="true"></pre>
            <textarea dir="rtl" spellcheck="false" autocomplete="off" autocapitalize="off"></textarea>
        </div>`;
    const gutter = root.querySelector('.gutter');
    const wrap = root.querySelector('.code-wrap');
    const highlight = root.querySelector('.highlight');
    const code = root.querySelector('textarea');
    code.placeholder = options.placeholder || 'اكتب الكود هنا...';
    code.value = options.value || '';

    let errorLine = null;

    function currentLineIndex() {
        return code.value.slice(0, code.selectionStart).split('\n').length - 1;
    }

    function render() {
        const lines = code.value.split('\n');
        const focused = document.activeElement === code;
        const active = focused ? currentLineIndex() : -1;
        highlight.innerHTML = lines.map((l, idx) => {
            const cls = ['line'];
            if (idx === active) cls.push('active');
            if (idx + 1 === errorLine) cls.push('error-line');
            return `<div class="${cls.join(' ')}">${highlightLine(l) || ' '}</div>`;
        }).join('');
        gutter.innerHTML = lines.map((_, idx) => {
            const cls = idx === active ? 'active' : idx + 1 === errorLine ? 'error' : '';
            return `<div class="${cls}">${toArabicDigits(idx + 1)}</div>`;
        }).join('');
        if (options.autoHeight) {
            const lineHeight = parseFloat(getComputedStyle(root).getPropertyValue('--editor-line')) || 26;
            const scrollbar = code.offsetHeight - code.clientHeight;
            wrap.style.height = `${lines.length * lineHeight + 24 + scrollbar}px`;
        }
        if (focused && options.onCursor) {
            const col = code.selectionStart - code.value.lastIndexOf('\n', code.selectionStart - 1);
            options.onCursor(active + 1, col);
        }
        syncScroll();
    }

    function syncScroll() {
        highlight.scrollTop = code.scrollTop;
        highlight.scrollLeft = code.scrollLeft;
        gutter.scrollTop = code.scrollTop;
    }

    // ---------- Editing helpers ----------
    function selectedLines() {
        const start = code.value.lastIndexOf('\n', code.selectionStart - 1) + 1;
        const end = code.selectionEnd;
        return { start, end, text: code.value.slice(start, end) };
    }

    function replaceRange(start, end, text, selStart, selEnd) {
        code.setSelectionRange(start, end);
        // execCommand keeps the browser's undo history working
        if (!document.execCommand('insertText', false, text)) code.setRangeText(text, start, end);
        const caret = selStart ?? start + text.length;
        code.setSelectionRange(caret, selEnd ?? caret);
    }

    function indentSelection(dedent) {
        const { start, end, text } = selectedLines();
        const changed = text.split('\n').map(l => dedent ? l.replace(/^ {1,4}/, '') : INDENT + l).join('\n');
        if (changed === text) return;
        if (code.selectionStart === code.selectionEnd) {
            const caret = Math.max(start, code.selectionStart + changed.length - text.length);
            replaceRange(start, end, changed, caret, caret);
        } else {
            replaceRange(start, end, changed, start, start + changed.length);
        }
    }

    function toggleComment() {
        const { start, end, text } = selectedLines();
        const lines = text.split('\n');
        const allCommented = lines.every(l => /^\s*#/.test(l) || !l.trim());
        const changed = lines.map(l => {
            if (!l.trim()) return l;
            return allCommented ? l.replace(/^(\s*)# ?/, '$1') : l.replace(/^(\s*)/, '$1# ');
        }).join('\n');
        replaceRange(start, end, changed, start, start + changed.length);
    }

    code.addEventListener('keydown', e => {
        const { selectionStart: s, selectionEnd: en, value: v } = code;

        if (e.key === 'Enter' && options.onRun) {
            const mode = (e.ctrlKey || e.metaKey) ? 'stay' : e.shiftKey ? 'next' : e.altKey ? 'insert' : null;
            if (mode) { e.preventDefault(); options.onRun(mode); return; }
        }
        if (e.key === '/' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); toggleComment(); return; }

        if (e.key === 'Tab') {
            e.preventDefault();
            if (e.shiftKey || s !== en) indentSelection(e.shiftKey);
            else replaceRange(s, en, INDENT);
            return;
        }

        // Auto-indent: keep current indentation, add a level after ':'
        if (e.key === 'Enter') {
            e.preventDefault();
            const lineStart = v.lastIndexOf('\n', s - 1) + 1;
            const before = v.slice(lineStart, s);
            let indent = before.match(/^ */)[0];
            const trimmed = before.replace(/#.*$/, '').trimEnd();
            if (trimmed.endsWith(':')) indent += INDENT;
            else if (/^\s*(أرجع|اكسر|استمرار|يمر|رفع)(?![\p{L}\p{M}\p{N}_])/u.test(before)) indent = indent.slice(4);
            replaceRange(s, en, '\n' + indent);
            return;
        }

        // Backspace inside indentation removes a whole level
        if (e.key === 'Backspace' && s === en && s > 0) {
            const lineStart = v.lastIndexOf('\n', s - 1) + 1;
            const before = v.slice(lineStart, s);
            if (before.length && /^ +$/.test(before)) {
                e.preventDefault();
                replaceRange(s - (before.length % 4 || 4), s, '');
                return;
            }
            if (PAIRS[v[s - 1]] && PAIRS[v[s - 1]] === v[s]) {
                e.preventDefault();
                replaceRange(s - 1, s + 1, '');
                return;
            }
        }

        // Skip over an auto-inserted closing character
        if ([')', ']', '}', '"', "'"].includes(e.key) && s === en && v[s] === e.key) {
            e.preventDefault();
            code.setSelectionRange(s + 1, s + 1);
            render();
            return;
        }

        // Auto-close brackets and quotes
        if (PAIRS[e.key] && !e.ctrlKey && !e.metaKey) {
            const isQuote = e.key === '"' || e.key === "'";
            if (isQuote && s === en && isIdent(v[s - 1] || '')) return;
            e.preventDefault();
            const inner = v.slice(s, en);
            replaceRange(s, en, e.key + inner + PAIRS[e.key], s + 1, s + 1 + inner.length);
        }
    });

    code.addEventListener('input', () => {
        errorLine = null;
        render();
        options.onChange?.();
    });
    code.addEventListener('scroll', syncScroll);
    code.addEventListener('focus', () => { render(); options.onFocus?.(); });
    ['blur', 'click', 'keyup', 'select'].forEach(ev => code.addEventListener(ev, render));
    document.addEventListener('selectionchange', () => { if (document.activeElement === code) render(); });

    render();

    return {
        textarea: code,
        get value() { return code.value; },
        set value(v) { code.value = v; errorLine = null; render(); },
        setErrorLine(n) { errorLine = n; render(); },
        focus(atEnd = false) {
            code.focus();
            if (atEnd) code.setSelectionRange(code.value.length, code.value.length);
        },
        render,
    };
}
