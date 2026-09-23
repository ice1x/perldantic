"""Extract the `all_errors` table from upstream pydantic-core tests into a JSON fixture.

Usage: python3 tools/extract_upstream_errors.py > crates/perldantic-core/tests/fixtures/upstream_error_messages.json

The table in upstream/pydantic-core/tests/test_errors.py lists every error type with an example
context and the message pydantic renders for it. Python exception objects used as context values
(ValueError('x'), AssertionError('x')) are reduced to their message string, which is what the
Python-free core stores.
"""

import ast
import json
import pathlib
import sys

SOURCE = pathlib.Path(__file__).resolve().parent.parent / 'upstream/pydantic-core/tests/test_errors.py'


class _ExceptionToString(ast.NodeTransformer):
    def visit_Call(self, node: ast.Call) -> ast.AST:
        if isinstance(node.func, ast.Name) and node.func.id.endswith('Error') and len(node.args) == 1:
            return node.args[0]
        return self.generic_visit(node)


def main() -> int:
    tree = ast.parse(SOURCE.read_text())
    for stmt in tree.body:
        if isinstance(stmt, ast.Assign) and any(getattr(t, 'id', None) == 'all_errors' for t in stmt.targets):
            value = _ExceptionToString().visit(stmt.value)
            rows = ast.literal_eval(ast.fix_missing_locations(value))
            cases = [{'type': t, 'message': m, 'context': c} for t, m, c in rows]
            json.dump(cases, sys.stdout, indent=1)
            sys.stdout.write('\n')
            return 0
    print('all_errors table not found', file=sys.stderr)
    return 1


if __name__ == '__main__':
    sys.exit(main())
