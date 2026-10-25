// Arabic → Python, a port of translate_to_python in arabic_translator.py.
// The GitHub Pages build runs code with the Arython interpreter, not Python; this
// is only for the "show Python" view and the .ipynb export.

const ARABIC_SYMBOLS = { '،': ',', '؛': ';', '٫': '.' };
for (let i = 0; i < 10; i++) {
    ARABIC_SYMBOLS[String.fromCharCode(0x0660 + i)] = String(i);  // ٠-٩
    ARABIC_SYMBOLS[String.fromCharCode(0x06F0 + i)] = String(i);  // ۰-۹ (Persian)
}

// Diacritics (shadda, harakat) are combining marks and belong to the word
const IDENT_CHAR = /^[_\p{L}\p{N}\p{M}ـ]$/u;
const DIGIT = /^\p{Nd}$/u;
const FSTRING_FIELD = /(?<!\{)\{([^{}]+)\}/g;

export function translateToPython(code, keywords = window.ARYTHON_KEYWORDS || {}) {
    const chars = Array.from(code.replace(/\r\n/g, '\n'));
    const n = chars.length;
    const startsWith = (s, at) => chars.slice(at, at + s.length).join('') === s;
    const out = [];
    let i = 0;
    while (i < n) {
        const ch = chars[i];

        // Comments: copy to end of line
        if (ch === '#') {
            let end = chars.indexOf('\n', i);
            if (end === -1) end = n;
            out.push(chars.slice(i, end).join(''));
            i = end;
            continue;
        }

        // Strings (single, double, and triple-quoted)
        if (ch === '"' || ch === "'") {
            const quote = startsWith('"""', i) ? '"""' : startsWith("'''", i) ? "'''" : ch;
            let j = i + quote.length;
            while (j < n && !startsWith(quote, j)) {
                if (chars[j] === '\\') j++;
                else if (quote.length === 1 && chars[j] === '\n') break;
                j++;
            }
            j = Math.min(j + quote.length, n);
            let literal = chars.slice(i, j).join('');
            // f-strings: translate the code inside {...}, but not the text around it
            if (out.length && ['f', 'rf', 'fr'].includes(out[out.length - 1].toLowerCase())) {
                literal = literal.replace(FSTRING_FIELD, (_, inner) => `{${translateToPython(inner, keywords)}}`);
            }
            out.push(literal);
            i = j;
            continue;
        }

        // Identifiers / keywords
        if (IDENT_CHAR.test(ch) && !DIGIT.test(ch)) {
            let j = i;
            while (j < n && IDENT_CHAR.test(chars[j])) j++;
            const word = chars.slice(i, j).join('');
            out.push(Object.hasOwn(keywords, word) ? keywords[word] : word);
            i = j;
            continue;
        }

        out.push(ARABIC_SYMBOLS[ch] ?? ch);
        i++;
    }
    return out.join('');
}
