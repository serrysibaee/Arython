<div dir="rtl" align="center">

# أرايثون · Arython

**اكتب بايثون بالعربية** — Write Python in Arabic

</div>

Arython is a small web app that lets you write Python programs using **Arabic keywords**. Code is typed in a right-to-left editor with syntax highlighting, translated to standard Python, and run on the server, showing both the output and the translated code.

<div dir="rtl">

```python
دالة مضروب(ن):
    إذا ن أصغر_أو_يساوي ١:
        أرجع ١
    أرجع ن * مضروب(ن - ١)

اطبع("مضروب ٥ =", مضروب(٥))
```

</div>

```
مضروب ٥ = 120
```

## Features

- **Arabic keywords and built-ins:** `اطبع` → `print`, `إذا` → `if`, `لكل` → `for`, `دالة` → `def`, `صنف` → `class`, and more.
- **Arabic digits and punctuation:** `١٢٣` and `،` `؛` work like `123` and `,` `;`.
- **Diacritics are fine:** `قرّب` and `قرب` are both read as whole words.
- **Text is left alone:** strings and comments are never translated, while code inside f-string braces (`f"{طول(س)}"`) is.
- **Right-to-left code editor:**
  - Line numbers in Arabic digits
  - Colors for keywords, built-ins, strings, numbers and comments
  - Dotted indentation guides
  - Highlighting of the current line and of the line where an error happened
  - Light and dark themes
- **Editing helpers:**
  - Indentation is kept on Enter, with a level added after `:`
  - Tab / Shift+Tab indent and un-indent selected lines
  - Ctrl+/ comments lines out and back in
  - Brackets and quotes close automatically
  - Ctrl+Enter runs the code
- **Program input:** a box for the text that `أدخل()` reads.
- **Built-in examples:** 19 example programs in 5 groups, from "Hello World" to classes and a guessing game.
- **Separate process:** each run happens in its own Python process with a 5-second time limit, and errors are shown in Arabic.

## Getting started

Requires Python 3.10+.

```bash
git clone https://github.com/<your-username>/Arython.git
cd Arython
pip install -r requirements.txt
uvicorn main:app --reload
```

Then open <http://127.0.0.1:8000>.

## Keyword reference

| العربية | Python | | العربية | Python |
|---|---|---|---|---|
| `اطبع` | `print` | | `صحيح` / `خطأ` | `True` / `False` |
| `أدخل` | `input` | | `لاشيء` | `None` |
| `إذا` | `if` | | `و` / `أو` / `غير` | `and` / `or` / `not` |
| `وإلا_إذا` | `elif` | | `يساوي` / `لا_يساوي` | `==` / `!=` |
| `وإلا` | `else` | | `أكبر_من` / `أصغر_من` | `>` / `<` |
| `لكل` … `في` | `for` … `in` | | `أكبر_أو_يساوي` / `أصغر_أو_يساوي` | `>=` / `<=` |
| `بينما` | `while` | | `طول` | `len` |
| `اكسر` / `استمرار` | `break` / `continue` | | `نطاق` | `range` |
| `دالة` / `أرجع` | `def` / `return` | | `عدد_صحيح` / `عدد_عشري` / `نص` | `int` / `float` / `str` |
| `صنف` | `class` | | `قائمة` / `قاموس` | `list` / `dict` |
| `حاول` / `عدا` / `أخيرا` | `try` / `except` / `finally` | | `مجموع` / `أكبر` / `أصغر` | `sum` / `max` / `min` |
| `رفع` | `raise` | | `مرتب` / `مطلق` / `قرّب` | `sorted` / `abs` / `round` |
| `استيراد` / `من` / `كـ` | `import` / `from` / `as` | | `عدّد` | `enumerate` |
| `مع` | `with` | | `أضف` | `.append` |
| `يمر` | `pass` | | `عناصر` / `مفاتيح` / `قيم` | `.items` / `.keys` / `.values` |
| `عالمي` / `غير_محلي` | `global` / `nonlocal` | | `افصل` / `ادمج` / `كبير` | `.split` / `.join` / `.upper` |
| `هو` | `is` | | `نوع` | `type` |

The full list is in `ARABIC_TO_PYTHON` in [`arabic_translator.py`](arabic_translator.py), and the app shows it under **الكلمات المتاحة**.

## How it works

1. `translate_to_python()` reads the code character by character. It copies strings and comments unchanged, swaps Arabic keywords for Python ones, and converts Arabic digits and punctuation. Because it works word by word, line numbers in error messages still match your Arabic code.
2. `run_arabic_code()` runs the translated code in a separate Python process (`python -I -X utf8 -c …`), passes it the program input, stops it after 5 seconds, and reduces errors to a short Arabic message with the line number.
3. The editor is a transparent `<textarea>` laid over a colored `<pre>`. It is plain HTML, CSS and JavaScript, with no external libraries.

## Project structure

```
├── main.py               # FastAPI app: page + /api/run endpoint
├── arabic_translator.py  # Keyword map, translator, and code runner
├── examples.py           # Example programs shown in the editor
├── templates/index.html  # Editor UI (RTL) and highlighter
└── static/style.css      # Styles and light/dark themes
```

## Adding keywords and examples

- **New keyword:** add `'العربية': 'python'` to `ARABIC_TO_PYTHON` in `arabic_translator.py`. If it is a built-in function or method, also add the Python name to `BUILTINS` in `templates/index.html` so it gets the built-in color.
- **New example:** add a `(key, title, code, stdin)` entry to a group in `examples.py`. It appears in the menu automatically.

## ⚠️ Security note

Code runs in a separate process with a time limit, but it is **not sandboxed**: it can read files, use the network and run commands with the server's permissions. Run Arython on your own machine or in a classroom, and don't expose it to the public internet without real isolation, such as a container with no network and a read-only file system.

## Known limitations

- Exception names (`ValueError`), `self` and `__init__` are still written in English.
- Python modules such as `random` and `math` keep their English names.
- An Arabic keyword can't be used as a variable name. For example, `نص` always means `str`.
