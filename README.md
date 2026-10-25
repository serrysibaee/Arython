<div dir="rtl" align="center">

# أرايثون · Arython

**اكتب بايثون بالعربية** — Write Python in Arabic

**[▶ Try it online](https://serrysibaee.github.io/Arython/)** · runs entirely in your browser

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

## Two ways to code

| | **Editor** (`/`) | **Interactive notebook** (`/notebook`) |
|---|---|---|
| Style | One program, run all at once | Colab/Jupyter-style cells |
| Variables | Fresh on every run | Kept between cells |
| Input (`أدخل`) | Typed in advance, one line per call | Asked for live in a box under the cell |
| Best for | Short programs, examples | Teaching, step-by-step exploration |

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
- **Interactive notebook:**
  - Code cells and text (Markdown) cells
  - One shared Python session, so variables persist between cells, and the last expression's value is shown
  - Run counters `[١]`, `[٢]`, and Run All / Stop / Restart kernel buttons
  - Live `أدخل()` prompts under the cell
  - Errors in Arabic, with the failing line highlighted in red
  - Shows the translated Python of each cell
  - Saved automatically in the browser; save and open `.arnb` files, or export to Jupyter `.ipynb` in Python
- **Built-in examples:** 19 example programs in 5 groups, from "Hello World" to classes and a guessing game.
- **Separate process:** each run happens in its own Python process with a 5-second time limit, and errors are shown in Arabic.

## Getting started

Requires Python 3.10+.

```bash
git clone https://github.com/serrysibaee/Arython.git
cd Arython
pip install -r requirements.txt
uvicorn main:app --reload
```

Then open <http://127.0.0.1:8000> for the editor, or <http://127.0.0.1:8000/notebook> for the notebook.

### Browser-only version (GitHub Pages)

The [online version](https://serrysibaee.github.io/Arython/) needs no server and no Python: code runs in the visitor's browser with Arython's own [interpreter](#standalone-interpreter-rust), compiled to WebAssembly (about 520 KB) and run in a Web Worker. It uses the same templates and editor as the server version. The "show Python" view and the `.ipynb` export come from `static/translate.js`, a JavaScript port of the translator.

Because the online version runs on the Rust interpreter, it supports what [that interpreter supports](#standalone-interpreter-rust), not all of Python. Every built-in example works.

- To build and preview it locally (needs [Rust](https://rustup.rs) for the WebAssembly build):

  ```bash
  pip install jinja2
  rustup target add wasm32-unknown-unknown
  python web/build.py                       # compiles compiler/ to WebAssembly, writes _site/
  python -m http.server -d _site 8000       # open http://localhost:8000
  ```

- To publish, push the contents of `_site/` to the `gh-pages` branch, which Pages serves (**Settings → Pages → Deploy from a branch → `gh-pages`**).

- Live `أدخل` input and the **Stop** button need shared memory, which browsers only allow on cross-origin-isolated pages. GitHub Pages can't set those headers, so `coi-serviceworker.js` adds them; the first visit reloads the page once.
- Very deep recursion can run out of the browser's own stack before reaching the limit of 1000 calls (recursion through a list comprehension stops at about 800 levels). The program then stops with a `RecursionError`, and in the notebook the kernel restarts.

## Standalone interpreter (Rust)

`compiler/` holds `arython`, a standalone interpreter for the language written in Rust. It doesn't need Python: one 680 KB executable with no dependencies that starts in about 6 ms.

```bash
cd compiler
cargo build --release            # builds target/release/arython(.exe)

arython برنامج.ary               # run a program
arython -c "اطبع(١ + ١)"          # run code directly
arython                          # interactive prompt (REPL)
```

It has its own lexer, parser and interpreter, and error messages are fully in Arabic:

```
⚠ نوع غير مناسب (TypeError): لا يمكن تطبيق '+' على نص وعدد_صحيح (حوّل الرقم إلى نص بـ نص() أو النص إلى رقم بـ عدد_صحيح())
  في السطر ٣
```

**What it supports:**
- Everything in the keyword table.
- Functions with default, `*args` and `**kwargs` parameters, closures, and `lambda`.
- Classes with inheritance, `super()` and special methods (`__str__`, `__eq__`, `__add__`, …).
- Exceptions, including your own exception classes.
- Lists, tuples, dicts, slices, comprehensions, f-strings with format specs, `str.format`, and `%` formatting.
- The `math`, `random` and `time` modules.

**Not supported yet:** sets, generators (`yield`), `with`, decorators, and other modules. Integers are 64-bit, so going past that limit raises `OverflowError` instead of growing like Python's.

**Testing:** `python compiler/tests/compare.py` runs every example in `examples.py`, plus the programs in `compiler/tests/cases/`, with both CPython and `arython`. It checks that the output is identical.

**Speed** compared with CPython 3.13 (whole programs, including startup):

| Program | CPython | arython |
|---|---|---|
| Recursive Fibonacci (27) | 0.09 s | 0.08 s |
| Arithmetic loop (3M iterations) | 0.30 s | 0.19 s |
| Lists, dicts, strings | 0.18 s | 0.16 s |
| Creating objects and calling methods | 0.15 s | 0.17 s |

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
3. **Notebook:** each browser tab opens a WebSocket (`/ws/notebook`) and gets its own kernel (`kernel.py`), a Python process that runs every cell in one shared namespace. **Stop** sends Ctrl+Break on Windows (SIGINT elsewhere), which interrupts the cell without losing variables; if the cell doesn't stop within 2 seconds, the kernel is restarted.
4. The editor is a transparent `<textarea>` laid over a colored `<pre>`. It is plain HTML, CSS and JavaScript, with no external libraries.

## Project structure

```
├── main.py                  # FastAPI app: pages, /api/run, /api/translate, /ws/notebook
├── arabic_translator.py     # Keyword map, translator, and one-shot code runner
├── kernel.py                # Notebook kernel process (shared namespace per notebook)
├── kernel_manager.py        # Starts kernels and relays messages over WebSocket
├── examples.py              # Example programs shown in the editor
├── compiler/                # Standalone interpreter in Rust (no Python needed)
│   ├── src/lexer.rs         # Arabic source → tokens (keyword table, digits, punctuation)
│   ├── src/parser.rs        # Tokens → syntax tree
│   ├── src/resolve.rs       # Assigns every variable a slot (local, closure cell or global)
│   ├── src/interp.rs        # Runs the syntax tree
│   ├── src/builtins.rs      # Built-in functions, methods, modules, formatting
│   ├── src/value.rs         # Runtime values
│   ├── src/alloc.rs         # Fast small-object allocator
│   ├── src/lib.rs           # Runs a piece of code; shared by the command line and the browser
│   ├── src/host.rs          # Output, input, clock and interrupts (terminal or browser)
│   ├── src/wasm.rs          # WebAssembly entry points for the Pages build
│   ├── src/main.rs          # Command line, REPL, Arabic error messages
│   └── tests/compare.py     # Checks the output matches CPython
├── web/                     # Browser-only build for GitHub Pages
│   ├── build.py             # Compiles the interpreter to WebAssembly, renders the static site in _site/
│   └── coi-serviceworker.js # Enables cross-origin isolation (third-party, MIT)
├── templates/
│   ├── index.html           # Single-file editor page
│   └── notebook.html        # Notebook page
└── static/
    ├── editor.js            # Shared RTL code editor + syntax highlighter
    ├── notebook.js          # Notebook cells, outputs, save/load, kernel connection
    ├── browser-runtime.js   # Pages build: in-browser kernel and editor runner
    ├── arython-worker.js    # Pages build: Web Worker that runs arython.wasm
    ├── translate.js         # Pages build: Arabic → Python for the "show Python" view and export
    └── style.css            # Styles and light/dark themes
```

## Adding keywords and examples

- **New keyword:** add `'العربية': 'python'` to `ARABIC_TO_PYTHON` in `arabic_translator.py`, and the same pair to `ARABIC_WORDS` in `compiler/src/lexer.rs`. If it is a built-in function or method, also add the Python name to `BUILTINS` in `static/editor.js` so it gets the built-in color.
- **New example:** add a `(key, title, code, stdin)` entry to a group in `examples.py`. It appears in the menu automatically.

## ⚠️ Security note

This applies to the server version. The online version is safe to share: code runs in each visitor's own browser sandbox.

On the server, code runs in a separate process (the editor also has a time limit; notebook cells run until stopped), but it is **not sandboxed**: it can read files, use the network and run commands with the server's permissions. Run Arython on your own machine or in a classroom, and don't expose it to the public internet without real isolation, such as a container with no network and a read-only file system.

## Known limitations

- Exception names (`ValueError`), `self` and `__init__` are still written in English.
- Python modules such as `random` and `math` keep their English names.
- An Arabic keyword can't be used as a variable name. For example, `نص` always means `str`.
