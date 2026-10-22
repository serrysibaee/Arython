"""Runs Arython programs with both CPython (via the translator) and the Rust binary,
and reports any difference in output.

Usage: python compiler/tests/compare.py [path/to/arython.exe]
Cases: every example in examples.py, plus compiler/tests/cases/*.ary
(an optional matching .in file supplies stdin).
"""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from arabic_translator import translate_to_python  # noqa: E402
from examples import EXAMPLE_GROUPS  # noqa: E402

EXE = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'compiler' / 'target' / 'release' / 'arython.exe'
CASES = Path(__file__).parent / 'cases'


def run_python(code, stdin):
    py = translate_to_python(code)
    r = subprocess.run([sys.executable, '-I', '-X', 'utf8', '-c', py], input=stdin, capture_output=True,
                       text=True, encoding='utf-8', timeout=30)
    if 'SyntaxError' in r.stderr:
        raise SystemExit(f'test program is not valid Python:\n{r.stderr}')
    return r.stdout, r.returncode


def run_arython(code, stdin):
    r = subprocess.run([str(EXE), '-c', code], input=stdin, capture_output=True, text=True,
                       encoding='utf-8', timeout=30)
    return r.stdout, r.returncode, r.stderr


def cases():
    for _, items in EXAMPLE_GROUPS:
        for key, _, code, stdin in items:
            yield f'example:{key}', code, stdin
    for path in sorted(CASES.glob('*.ary')):
        stdin_path = path.with_suffix('.in')
        stdin = stdin_path.read_text(encoding='utf-8') if stdin_path.exists() else ''
        yield f'case:{path.stem}', path.read_text(encoding='utf-8'), stdin


def main():
    failed = 0
    total = 0
    for name, code, stdin in cases():
        total += 1
        want, want_code = run_python(code, stdin)
        if want_code != 0 and 'errors' not in name:
            raise SystemExit(f'{name}: fails in Python too; only errors_* cases may end in an error')
        got, got_code, err = run_arython(code, stdin)
        # Programs that end in an error must print the same output before it
        if want == got and (want_code == 0) == (got_code == 0):
            print(f'ok    {name}')
            continue
        failed += 1
        print(f'FAIL  {name}')
        print('  python :', repr(want), want_code)
        print('  arython:', repr(got), got_code)
        if err:
            print('  stderr :', err.strip())
    print(f'\n{total - failed}/{total} passed')
    sys.exit(1 if failed else 0)


if __name__ == '__main__':
    main()
