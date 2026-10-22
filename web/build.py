"""Builds the browser-only site for GitHub Pages into _site/.

Python runs in the visitor's browser with Pyodide, so no server is needed.
Usage: python web/build.py   (then serve _site/ with any static file server)
"""
import shutil
import sys
from pathlib import Path

from jinja2 import Environment, FileSystemLoader

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / 'web'
OUT = ROOT / '_site'
sys.path.insert(0, str(ROOT))

from arabic_translator import ARABIC_TO_PYTHON  # noqa: E402
from examples import EXAMPLE_GROUPS  # noqa: E402

# Python files the in-browser runtime loads (see static/pyworker.js)
PY_FILES = [ROOT / 'arabic_translator.py', ROOT / 'kernel.py', WEB / 'browser_runtime.py']
PAGE_CONTEXT = {'mode': 'browser', 'home_url': './', 'notebook_url': 'notebook.html', 'static_url': './static'}
PAGES = {
    'index.html': {'keywords': ARABIC_TO_PYTHON, 'example_groups': EXAMPLE_GROUPS},
    'notebook.html': {'keywords': ARABIC_TO_PYTHON},
}


def main():
    shutil.rmtree(OUT, ignore_errors=True)
    shutil.copytree(ROOT / 'static', OUT / 'static')
    (OUT / 'py').mkdir()
    for path in PY_FILES:
        shutil.copy(path, OUT / 'py' / path.name)
    # Must sit at the site root so its scope covers every page
    shutil.copy(WEB / 'coi-serviceworker.js', OUT)
    (OUT / '.nojekyll').touch()

    env = Environment(loader=FileSystemLoader(ROOT / 'templates'), autoescape=True)
    for name, context in PAGES.items():
        html = env.get_template(name).render(**PAGE_CONTEXT, **context)
        (OUT / name).write_text(html, encoding='utf-8')
    print(f'Built {OUT.relative_to(ROOT)}')


if __name__ == '__main__':
    main()
