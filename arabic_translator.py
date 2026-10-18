import re
import subprocess
import unicodedata
import sys

ARABIC_TO_PYTHON = {
    # Keywords
    'اطبع': 'print',
    'إذا': 'if',
    'وإلا_إذا': 'elif',
    'وإلا': 'else',
    'بينما': 'while',
    'لكل': 'for',
    'في': 'in',
    'في_ضمن': 'in',
    'دالة': 'def',
    'أرجع': 'return',
    'استمرار': 'continue',
    'اكسر': 'break',
    'حاول': 'try',
    'عدا': 'except',
    'أخيرا': 'finally',
    'مع': 'with',
    'كـ': 'as',
    'استيراد': 'import',
    'من': 'from',
    'صحيح': 'True',
    'خطأ': 'False',
    'لاشيء': 'None',
    'و': 'and',
    'أو': 'or',
    'غير': 'not',
    'صنف': 'class',
    'يمر': 'pass',
    'عالمي': 'global',
    'غير_محلي': 'nonlocal',
    'رفع': 'raise',
    'هو': 'is',
    # Built-in functions
    'أدخل': 'input',
    'طول': 'len',
    'نوع': 'type',
    'نطاق': 'range',
    'عدد_صحيح': 'int',
    'عدد_عشري': 'float',
    'نص': 'str',
    'قائمة': 'list',
    'مجموع': 'sum',
    'أكبر': 'max',
    'أصغر': 'min',
    'مطلق': 'abs',
    'قرّب': 'round',
    'مرتب': 'sorted',
    'قاموس': 'dict',
    'عدّد': 'enumerate',
    # Common methods
    'أضف': 'append',
    'عناصر': 'items',
    'مفاتيح': 'keys',
    'قيم': 'values',
    'افصل': 'split',
    'ادمج': 'join',
    'كبير': 'upper',
    # Word operators
    'يساوي': '==',
    'لا_يساوي': '!=',
    'أكبر_من': '>',
    'أصغر_من': '<',
    'أكبر_أو_يساوي': '>=',
    'أصغر_أو_يساوي': '<=',
}

# Arabic punctuation and digits, converted only outside strings and comments
ARABIC_SYMBOLS = {
    '،': ',', '؛': ';', '٫': '.',
    **{chr(0x0660 + i): str(i) for i in range(10)},  # ٠-٩
    **{chr(0x06F0 + i): str(i) for i in range(10)},  # ۰-۹ (Persian)
}

TIMEOUT_SECONDS = 5


def _is_ident_char(ch):
    # Diacritics (shadda, harakat) are combining marks and belong to the word
    return ch == '_' or ch.isalnum() or ch == 'ـ' or unicodedata.category(ch).startswith('M')


def translate_to_python(arabic_code):
    """Replace Arabic keywords with Python ones, leaving strings and comments untouched."""
    out = []
    i, n = 0, len(arabic_code)
    while i < n:
        ch = arabic_code[i]

        # Comments: copy to end of line
        if ch == '#':
            end = arabic_code.find('\n', i)
            end = n if end == -1 else end
            out.append(arabic_code[i:end])
            i = end
            continue

        # Strings (single, double, and triple-quoted)
        if ch in '"\'':
            quote = arabic_code[i:i + 3] if arabic_code[i:i + 3] in ('"""', "'''") else ch
            j = i + len(quote)
            while j < n and not arabic_code.startswith(quote, j):
                if arabic_code[j] == '\\':
                    j += 1
                elif len(quote) == 1 and arabic_code[j] == '\n':
                    break
                j += 1
            j = min(j + len(quote), n)
            literal = arabic_code[i:j]
            # f-strings: translate the code inside {...}, but not the text around it
            if out and out[-1].lower() in ('f', 'rf', 'fr'):
                literal = re.sub(r'(?<!\{)\{([^{}]+)\}',
                                 lambda m: '{' + translate_to_python(m.group(1)) + '}', literal)
            out.append(literal)
            i = j
            continue

        # Identifiers / keywords
        if _is_ident_char(ch) and not ch.isdigit():
            j = i
            while j < n and _is_ident_char(arabic_code[j]):
                j += 1
            word = arabic_code[i:j]
            out.append(ARABIC_TO_PYTHON.get(word, word))
            i = j
            continue

        out.append(ARABIC_SYMBOLS.get(ch, ch))
        i += 1
    return ''.join(out)


def run_arabic_code(arabic_code, stdin_text=''):
    """Translate and run the code in a separate process with a time limit."""
    python_code = translate_to_python(arabic_code.replace('\r\n', '\n'))
    try:
        proc = subprocess.run(
            [sys.executable, '-I', '-X', 'utf8', '-c', python_code],
            input=stdin_text,
            capture_output=True,
            text=True,
            encoding='utf-8',
            timeout=TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        return python_code, f'خطأ: تجاوز البرنامج الحد الزمني ({TIMEOUT_SECONDS} ثوانٍ).', True

    output = proc.stdout
    if proc.returncode != 0:
        lines = proc.stderr.strip().splitlines()
        error = lines[-1] if lines else 'خطأ غير معروف'
        line_info = next((l.strip() for l in reversed(lines) if 'line ' in l), '')
        output += f'خطأ في التنفيذ: {error}' + (f'\n({line_info})' if line_info else '')
    return python_code, output, proc.returncode != 0
