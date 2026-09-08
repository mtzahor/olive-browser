#![cfg(feature = "js")]
use olive_html::js::{DocumentSession, ErrorKind, Runtime, ScriptOptions, run_document};
use olive_html::{Document, NodeId, NodeKind, ParseOptions, parse, parse_utf8};
use std::{collections::HashSet, io};

fn runtime() -> Runtime {
    Runtime::new(ScriptOptions::default()).unwrap()
}
fn find(doc: &Document, id: &str) -> Option<NodeId> {
    doc.descendants(doc.root()).find(|&n| {
        doc.node(n)
            .unwrap()
            .as_element()
            .is_some_and(|e| e.attribute("id") == Some(id))
    })
}
fn text(doc: &Document, id: &str) -> String {
    doc.descendants(find(doc, id).unwrap())
        .filter_map(|n| match &doc.node(n).unwrap().kind {
            NodeKind::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}
fn run(source: &str) -> (Document, olive_html::js::ScriptReport) {
    let doc = parse_utf8(
        source.as_bytes(),
        ParseOptions {
            scripting_enabled: true,
            ..Default::default()
        },
    )
    .unwrap()
    .document;
    run_document(doc, ScriptOptions::default())
}
fn integrity(doc: &Document) {
    let mut seen = HashSet::new();
    let mut pending = vec![doc.root()];
    while let Some(parent) = pending.pop() {
        assert!(seen.insert(parent));
        for child in doc.children(parent).take(doc.node_count() + 1) {
            assert_eq!(doc.node(child).unwrap().parent(), Some(parent));
            pending.push(child);
        }
    }
}
#[test]
fn parsing_is_inert_and_programs_belong_to_their_runtime() {
    let mut a = runtime();
    let program = a
        .parse("globalThis.answer = 42; console.log(answer); answer")
        .unwrap();
    assert_eq!(a.eval("typeof answer").unwrap(), "undefined");
    assert!(a.console().is_empty());
    assert_eq!(
        runtime().execute(&program).unwrap_err().kind,
        ErrorKind::WrongRuntime
    );
    assert_eq!(a.execute(&program).unwrap(), "42");
    assert_eq!(a.console(), ["42"]);
    assert_eq!(a.eval("answer + 1").unwrap(), "43");
    assert_eq!(runtime().eval("typeof answer").unwrap(), "undefined");
}
#[test]
fn language_semantics_come_from_the_ecmascript_engine() {
    let mut r = runtime();
    for (source, expected) in [
        ("1 + 2 * 3 ** 2", "19"),
        ("[1,2,3].map(x => x * 2).join(',')", "2,4,6"),
        (
            "function counter(){let n=0;return ()=>++n} const next=counter(); next();next()",
            "2",
        ),
        (
            "let sum=0;for(let i=0;i<5;i++){if(i===3)continue;sum+=i}sum",
            "7",
        ),
        ("const {a,b=3}={a:4}; `${a+b}!`", "7!"),
        (
            "class A { constructor(x){this.x=x} twice(){return this.x*2} } new A(6).twice()",
            "12",
        ),
        ("try { throw new Error('x') } catch(e) { e.message }", "x"),
        (
            "JSON.stringify({ok: true, n: null})",
            "{\"ok\":true,\"n\":null}",
        ),
        ("String(10n ** 3n)", "1000"),
        ("'olive'.replace(/o/g, 'O')", "Olive"),
        ("let x=1; {let x=2;} x", "1"),
    ] {
        assert_eq!(r.eval(source).unwrap(), expected, "{source}");
    }
}
#[test]
fn syntax_and_runtime_errors_are_distinct_and_recoverable() {
    let mut r = runtime();
    assert_eq!(r.eval("let = ;").unwrap_err().kind, ErrorKind::Syntax);
    assert_eq!(r.eval("missing()").unwrap_err().kind, ErrorKind::Runtime);
    assert_eq!(r.eval("40+2").unwrap(), "42");
    assert!(r.eval("const duplicate=1; const duplicate=2").is_err());
    assert!(r.eval("'use strict'; accidentalGlobal=1").is_err());
}
#[test]
fn unavailable_host_apis_and_dynamic_code_do_not_gain_capabilities() {
    let mut r = runtime();
    assert_eq!(r.eval("[typeof fetch,typeof require,typeof process,typeof setTimeout,typeof document].join(',')").unwrap(), "undefined,undefined,undefined,undefined,undefined");
    for s in [
        "eval('1')",
        "new Function('return 1')",
        "(()=>{}).constructor('return 1')()",
        "new (Object.getPrototypeOf(async function(){}).constructor)('return 1')",
    ] {
        assert!(r.eval(s).is_err(), "{s}");
    }
    assert_eq!(
        r.eval("let job=false; Promise.resolve().then(()=>job=true); job")
            .unwrap(),
        "false"
    );
    assert_eq!(r.eval("job").unwrap(), "false");
    assert_eq!(
        r.eval(
            "let imported=false; import('file:///etc/passwd').then(()=>imported=true); imported"
        )
        .unwrap(),
        "false"
    );
}
#[test]
fn console_and_results_are_bounded_and_do_not_invoke_user_code() {
    let mut r = Runtime::new(ScriptOptions {
        max_messages: 2,
        max_output_bytes: 8,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        r.eval("let touched=false; const o={toString(){touched=true;throw 1}}; console.log(o); o")
            .unwrap(),
        "[object]"
    );
    assert_eq!(r.eval("touched").unwrap(), "false");
    assert_eq!(
        r.eval("console.log('🫒'.repeat(20)); console.log('hidden'); '🫒'.repeat(20)")
            .unwrap(),
        "🫒🫒"
    );
    assert_eq!(r.console(), ["[object]", "🫒🫒"]);
    assert_eq!(r.omitted_console_messages(), 1);
}
#[test]
fn source_and_reader_limits_and_utf8_errors() {
    let mut r = Runtime::new(ScriptOptions {
        max_input_bytes: 8,
        ..Default::default()
    })
    .unwrap();
    assert!(r.parse("'🫒'").is_ok());
    assert_eq!(r.parse("'🫒🫒'").unwrap_err().kind, ErrorKind::Limit);
    let mut endless = io::repeat(b' ');
    assert_eq!(
        r.parse_reader(&mut endless).unwrap_err().kind,
        ErrorKind::Limit
    );
    assert_eq!(
        r.parse_reader(&b"\xff"[..]).unwrap_err().kind,
        ErrorKind::Input
    );
    struct Broken;
    impl io::Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("failed"))
        }
    }
    assert_eq!(r.parse_reader(Broken).unwrap_err().kind, ErrorKind::Input);
    assert!(r.parse_reader(&b"\xef\xbb\xbf1+2"[..]).is_ok());
}
#[test]
fn execution_and_recursion_limits_stop_runaway_code() {
    for code in [
        "while(true){}",
        "for(;;){}",
        "do{}while(true)",
        "function f(){f()} f()",
        "try { while(true){} } catch(e) { while(true){} }",
    ] {
        let mut r = Runtime::new(ScriptOptions {
            max_instructions: 10_000,
            max_loop_iterations: 1_000,
            max_recursion: 16,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(r.eval(code).unwrap_err().kind, ErrorKind::Limit, "{code}");
        assert_eq!(r.eval("1").unwrap_err().kind, ErrorKind::Limit);
    }
    let mut r = Runtime::new(ScriptOptions {
        max_instructions: 0,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(r.eval("1").unwrap_err().kind, ErrorKind::Limit);
}
#[test]
fn deep_and_complex_sources_are_rejected_before_the_recursive_parser() {
    for source in [
        format!("{}1{}", "(".repeat(8000), ")".repeat(8000)),
        "1+".repeat(8000),
        "!".repeat(8000),
        "typeof ".repeat(8000),
        "new ".repeat(8000),
    ] {
        assert_eq!(runtime().parse(&source).unwrap_err().kind, ErrorKind::Limit);
    }
}
#[test]
fn inline_scripts_share_state_update_the_dom_and_continue_after_errors() {
    let (doc, report) = run(
        r#"<!doctype html><title>Before</title><p id=result>before</p>
        <script>const values=[2,3,4]; document.getElementById('result').textContent=values.reduce((a,b)=>a+b,0); console.log('sum',9);</script>
        <script>let = ;</script><script>throw 'oops'</script>
        <script>document.title='After'; document.getElementById('result').setAttribute('class','done'); console.log(values.length)</script>"#,
    );
    assert_eq!(report.executed, 2);
    assert_eq!(report.attempted, 4);
    assert_eq!(report.diagnostics.len(), 2);
    assert_eq!(report.console, ["sum 9", "3"]);
    assert_eq!(text(&doc, "result"), "9");
    assert_eq!(
        doc.node(find(&doc, "result").unwrap())
            .unwrap()
            .as_element()
            .unwrap()
            .attribute("class"),
        Some("done")
    );
    integrity(&doc);
}
#[test]
fn creating_moving_removing_nodes_preserves_identity_and_links() {
    let (doc, report) = run(
        r#"<!doctype html><div id=a><b id=old>Old</b></div><div id=b></div><script>
        const a=document.getElementById('a'), b=document.getElementById('b');
        const old=document.getElementById('old'); a.textContent='Replaced';
        console.log(old.parentNode===null, old.textContent);
        b.appendChild(old); console.log(old === document.getElementById('old'));
        const p=document.createElement('P'); p.id='new'; p.className='note';
        p.appendChild(document.createTextNode('Created')); b.appendChild(p);
        console.log(p.parentNode===b, p.tagName, p.getAttribute('missing'));
        p.setAttribute('hidden',''); p.removeAttribute('HIDDEN');
        try { p.appendChild(b) } catch(e) { console.log('cycle rejected') }
        old.remove();
        </script>"#,
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(
        report.console,
        ["true Old", "true", "true P null", "cycle rejected"]
    );
    assert_eq!(text(&doc, "a"), "Replaced");
    assert_eq!(text(&doc, "b"), "Created");
    assert!(find(&doc, "old").is_none());
    integrity(&doc);
}
#[test]
fn only_connected_supported_inline_classic_scripts_execute() {
    let (_, report) = run(r#"<!doctype html><script src='missing.js'>throw 1</script>
        <script type=module>throw 2</script><script type=application/json>{"x":1}</script>
        <template><script>throw 3</script></template><noscript><script>throw 4</script></noscript>
        <svg><script>throw 5</script><foreignObject><script>throw 6</script></foreignObject></svg>
        <script type=" text/javascript ">console.log('classic')</script>
        <script language=javascript>console.log('legacy')</script>
        <script type='text/javascript; charset=utf-8'>throw 7</script>
        <script nomodule>console.log('nomodule')</script>"#);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.console, ["classic", "legacy", "nomodule"]);
    assert_eq!(report.skipped, 4);
}
#[test]
fn scripts_observe_full_document_and_removed_or_new_scripts_do_not_run() {
    let (_, report) = run(r#"<!doctype html><script>
        document.getElementById('later').remove();
        const script=document.createElement('script'); script.textContent="console.log('dynamic')";
        document.body.appendChild(script); console.log(document.getElementById('end').textContent);
        </script><script id=later>throw 1</script><p id=end>Full DOM</p>"#);
    assert!(report.diagnostics.is_empty());
    assert_eq!(report.console, ["Full DOM"]);
    assert_eq!(report.attempted, 1);
}
#[test]
fn document_resource_limits_retain_partial_changes_and_stop_later_scripts() {
    let doc = parse("<p id=p>before</p><script>document.getElementById('p').textContent='saved'; while(true){}</script><script>document.getElementById('p').textContent='wrong'</script>").unwrap().document;
    let (doc, report) = run_document(
        doc,
        ScriptOptions {
            max_instructions: 10_000,
            ..Default::default()
        },
    );
    assert!(report.limited);
    assert_eq!(report.attempted, 1);
    assert_eq!(text(&doc, "p"), "saved");
    for options in [
        ScriptOptions {
            max_dom_bytes: 1,
            ..Default::default()
        },
        ScriptOptions {
            max_dom_operations: 0,
            ..Default::default()
        },
        ScriptOptions {
            max_new_nodes: 0,
            ..Default::default()
        },
    ] {
        let doc = parse("<p id=p>before</p><script>try { document.getElementById('p').textContent='after' } catch(e) {}</script>").unwrap().document;
        let (doc, report) = run_document(doc, options);
        assert!(report.limited, "{options:?}");
        assert_eq!(text(&doc, "p"), "before");
        integrity(&doc);
    }
}
#[test]
fn document_source_script_and_diagnostic_budgets() {
    for options in [
        ScriptOptions {
            max_input_bytes: 1,
            ..Default::default()
        },
        ScriptOptions {
            max_scripts: 1,
            ..Default::default()
        },
    ] {
        let doc = parse("<script>1;</script><script>2;</script>")
            .unwrap()
            .document;
        let (_, report) = run_document(doc, options);
        assert!(report.limited);
    }
    let doc = parse("<script>throw 1</script><script>throw 2</script>")
        .unwrap()
        .document;
    let (_, report) = run_document(
        doc,
        ScriptOptions {
            max_messages: 1,
            ..Default::default()
        },
    );
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.omitted_diagnostics, 1);
}
#[test]
fn html_parsing_remains_inert_and_scripting_flag_only_changes_noscript_parsing() {
    let doc = parse(
        "<p id=p>before</p><script>document.getElementById('p').textContent='after'</script>",
    )
    .unwrap()
    .document;
    assert_eq!(text(&doc, "p"), "before");
    let source = "<!doctype html><body><noscript><p id=fallback>fallback</p></noscript>";
    assert!(find(&parse(source).unwrap().document, "fallback").is_some());
    let doc = parse_utf8(
        source.as_bytes(),
        ParseOptions {
            scripting_enabled: true,
            ..Default::default()
        },
    )
    .unwrap()
    .document;
    assert!(find(&doc, "fallback").is_none());
}
#[test]
fn deterministic_malformed_javascript_returns_errors_without_panics() {
    let tokens = [
        "let", "const", "x", "{", "}", ";", "=", "1", "'", "(", ")", "return", "=>", "🫒", "/*",
        "/",
    ];
    let mut state = 13u64;
    let mut r = runtime();
    for _ in 0..200 {
        let mut source = String::new();
        for _ in 0..30 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            source.push_str(tokens[(state >> 32) as usize % tokens.len()]);
            source.push(' ');
        }
        let _ = r.parse(&source);
    }
}

#[test]
fn inline_click_handlers_call_alert_and_return_false() {
    let source =
        "<!doctype html><a id=button href='#' onclick=\"alert('Hello!'); return false;\">Click</a>";
    let parsed = parse_utf8(
        source.as_bytes(),
        ParseOptions {
            scripting_enabled: true,
            ..Default::default()
        },
    )
    .unwrap();
    let (document, report) = run_document(parsed.document, ScriptOptions::default());
    let mut session = DocumentSession::attach(document, ScriptOptions::default(), report);
    let target = session.with_document(|doc| find(doc, "button")).unwrap();
    assert!(!session.click(target));
    assert_eq!(session.take_alerts(), ["Hello!"]);
    assert!(
        session.report().diagnostics.is_empty(),
        "{:?}",
        session.report().diagnostics
    );
}

#[test]
fn instruction_budget_covers_native_callbacks_and_multiple_scripts() {
    let mut r = Runtime::new(ScriptOptions {
        max_instructions: 1000,
        ..Default::default()
    })
    .unwrap();
    let source = "[0].forEach(()=>{let n=0;for(let i=0;i<100;i++){for(let j=0;j<100;j++){n++}}})";
    assert_eq!(r.eval(source).unwrap_err().kind, ErrorKind::Limit);
    let doc = parse(&"<script>for(let i=0;i<40;i++){};</script>".repeat(8))
        .unwrap()
        .document;
    let (_, report) = run_document(
        doc,
        ScriptOptions {
            max_instructions: 1000,
            ..Default::default()
        },
    );
    assert!(report.limited);
    assert!(report.attempted < 8);
}

#[test]
fn preloaded_scripts_share_limits_and_ignore_changed_sources() {
    use olive_html::{ExternalSource, js::DocumentSession};
    use std::collections::HashMap;
    let document = parse("<script>document.getElementById('external').setAttribute('src','changed.js')</script><script id=external src=original.js></script><script>console.log('after')</script>").unwrap().document;
    let id = document
        .descendants(document.root())
        .find(|&id| {
            document
                .node(id)
                .unwrap()
                .as_element()
                .is_some_and(|e| e.attribute("id") == Some("external"))
        })
        .unwrap();
    let sources = HashMap::from([(
        id,
        ExternalSource {
            reference: "original.js".into(),
            source: "throw 'stale'".into(),
        },
    )]);
    let session = DocumentSession::with_sources(document, ScriptOptions::default(), &sources);
    assert_eq!(session.report().console, ["after"]);
    assert_eq!(session.report().skipped, 1);
    assert!(session.report().diagnostics.is_empty());

    for options in [
        ScriptOptions {
            max_input_bytes: 3,
            ..ScriptOptions::default()
        },
        ScriptOptions {
            max_scripts: 1,
            ..ScriptOptions::default()
        },
    ] {
        let document = parse("<script>1;</script><script src=app.js></script>")
            .unwrap()
            .document;
        let id = document
            .descendants(document.root())
            .find(|&id| {
                document
                    .node(id)
                    .unwrap()
                    .as_element()
                    .is_some_and(|e| e.attribute("src").is_some())
            })
            .unwrap();
        let sources = HashMap::from([(
            id,
            ExternalSource {
                reference: "app.js".into(),
                source: "2;".into(),
            },
        )]);
        let session = DocumentSession::with_sources(document, options, &sources);
        assert_eq!(session.report().executed, 1);
        assert!(session.report().limited);
    }
}

#[test]
fn independent_statements_and_literal_data_do_not_exhaust_expression_budget() {
    let mut source = "let sum=0;".to_owned();
    source.push_str(&"sum+=1;".repeat(1200));
    source.push_str("sum");
    assert_eq!(runtime().eval(&source).unwrap(), "1200");
    let source = format!("'{}'.length", "new []?!;".repeat(2000));
    assert_eq!(runtime().eval(&source).unwrap(), "18000");
    let source = format!("/* {} */ 1", "if {{{ !".repeat(2000));
    assert_eq!(runtime().eval(&source).unwrap(), "1");
    for source in [
        "if(true);else ".repeat(3000),
        format!("{}1{}", "(\")))\", ".repeat(100), ")".repeat(100)),
        format!("/* ignored */ {}1", "typeof ".repeat(100)),
        format!("`value ${{{}1}}`", "!".repeat(100)),
    ] {
        assert_eq!(runtime().parse(&source).unwrap_err().kind, ErrorKind::Limit);
    }
}

#[test]
fn document_storage_is_functional_bounded_and_isolated() {
    let (_, report) = run(r#"<script>
        localStorage.setItem('__proto__', 'safe');
        localStorage.setItem('count', 7);
        sessionStorage.setItem('count', 3);
        console.log(window.localStorage === localStorage, localStorage.length, localStorage.key(0));
        console.log(localStorage.getItem('count'), sessionStorage.getItem('count'), localStorage.getItem('missing'));
        localStorage.setItem('count', 8);
        console.log(localStorage.length, localStorage.getItem('__proto__'), localStorage.getItem('count'));
        try { localStorage.setItem('count', 'x'.repeat(65536)); } catch(e) { console.log('quota') }
        console.log(localStorage.getItem('count'));
        localStorage.removeItem('__proto__');
        console.log(localStorage.key(0), localStorage.key(1));
        localStorage.clear();
        console.log(localStorage.length, sessionStorage.length);
    </script>"#);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(
        report.console,
        [
            "true 2 __proto__",
            "7 3 null",
            "2 safe 8",
            "quota",
            "8",
            "count null",
            "0 1"
        ]
    );
    let (_, report) =
        run("<script>console.log(localStorage.length, sessionStorage.length)</script>");
    assert_eq!(report.console, ["0 0"]);
    let (_, report) = run(r#"<script>
        let conversions=0;
        try { localStorage.setItem({toString(){conversions++;return 'x'}}, 'y') } catch(e) {}
        console.log(conversions, localStorage.length);
        for(let i=0;i<128;i++) localStorage.setItem('k'+i, 'v');
        try { localStorage.setItem('overflow', 'v') } catch(e) { console.log('keys bounded') }
        console.log(localStorage.length);
    </script>"#);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.console, ["0 0", "keys bounded", "128"]);
}

#[test]
fn navigator_and_tag_collection_support_common_bootstrap_scripts() {
    let (document, report) = run(r#"<head><script id=first>
        console.log(navigator === window.navigator, navigator.language, navigator.languages[0], navigator.cookieEnabled);
        console.log(/iphone|ipod|android/.test(navigator.userAgent.toLowerCase()));
        var scripts=document.getElementsByTagName('SCRIPT');
        var first=scripts[0], inserted=document.createElement('script');
        inserted.id='inserted'; inserted.textContent="throw 'dynamic must not run'";
        first.parentNode.insertBefore(inserted, first);
        console.log(scripts.length, document.head.getElementsByTagName('script').length);
        document.head.insertBefore(first, first);
    </script></head><body><p id=p>Ready</p><script>
        try { document.getElementById('p').insertBefore(document.createElement('b'), first) } catch(e) {console.log('wrong parent')}
    </script>"#);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.executed, 2);
    assert_eq!(
        report.console,
        ["true en-US en-US false", "false", "2 2", "wrong parent"]
    );
    integrity(&document);
}
