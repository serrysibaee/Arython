"""Builds the browser-only site for GitHub Pages into _site/.

Code runs in the visitor's browser with the Arython interpreter (compiler/) compiled
to WebAssembly, so no server is needed.
Usage: python web/build.py   (then serve _site/ with any static file server)
Needs Rust with the wasm32-unknown-unknown target (rustup target add wasm32-unknown-unknown).
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

from jinja2 import Environment, FileSystemLoader

ROOT = Path(__file__).resolve().parent.parent
WEB = ROOT / 'web'
OUT = ROOT / '_site'
COMPILER = ROOT / 'compiler'
WASM_TARGET = 'wasm32-unknown-unknown'
sys.path.insert(0, str(ROOT))

from arabic_translator import ARABIC_TO_PYTHON  # noqa: E402
from examples import EXAMPLE_GROUPS  # noqa: E402

PAGE_CONTEXT = {'mode': 'browser', 'home_url': './', 'notebook_url': 'notebook.html', 'static_url': './static'}
PAGES = {
    'index.html': {'keywords': ARABIC_TO_PYTHON, 'example_groups': EXAMPLE_GROUPS},
    'notebook.html': {'keywords': ARABIC_TO_PYTHON},
}


def build_wasm():
    """Compiles the interpreter to WebAssembly; returns the path of the .wasm file."""
    cargo = shutil.which('cargo') or str(Path.home() / '.cargo' / 'bin' / 'cargo')
    if not os.path.exists(cargo) and not shutil.which('cargo'):
        raise SystemExit('cargo not found: install Rust from https://rustup.rs')
    subprocess.run([cargo, 'rustc', '--lib', '--release', '--target', WASM_TARGET, '--crate-type', 'cdylib'],
                   cwd=COMPILER, check=True)
    return COMPILER / 'target' / WASM_TARGET / 'release' / 'arython.wasm'


def main():
    wasm = build_wasm()
    shutil.rmtree(OUT, ignore_errors=True)
    shutil.copytree(ROOT / 'static', OUT / 'static')
    shutil.copy(wasm, OUT / 'static' / 'arython.wasm')
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
