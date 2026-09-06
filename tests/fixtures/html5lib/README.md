# HTML tree-construction fixtures

`tests1.dat` is an unmodified copy from
[html5lib-tests at 9329e64694e7835d0dcff9811e22856ef6ad16f9](https://github.com/html5lib/html5lib-tests/tree/9329e64694e7835d0dcff9811e22856ef6ad16f9/tree-construction).
Its MIT license is included in `LICENSE`.

This revision predates the move of tree-construction tests to Web Platform Tests.
The harness checks every document tree in this file, including attribute values
and child order. Error-message strings and source locations are not compared:
html5ever uses different diagnostic wording. This is a regression subset, not a
claim that Olive passes the entire web-platform test suite.
