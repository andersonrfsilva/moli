use super::*;

const FETCH_HELPERS_SLOT: &str = "__moliFetchBodyStreamHelpers";

fn fetch_helpers_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    crate::util::get_private_value(scope, global, FETCH_HELPERS_SLOT)
}

fn default_fetch_helpers(vm: &mut ScriptVm) -> Option<v8::Global<v8::Value>> {
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        Ok(fetch_helpers_in_scope(scope).map(|value| v8::Global::new(scope, value)))
    })
    .expect("Fetch helper diagnostics")
}

fn fetch_helpers_in_context(
    vm: &mut ScriptVm,
    context_ptr: *const v8::Global<v8::Context>,
) -> Option<v8::Global<v8::Value>> {
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(context_ptr, |scope, _| {
        Ok(fetch_helpers_in_scope(scope).map(|value| v8::Global::new(scope, value)))
    })
    .expect("realm-local Fetch helper diagnostics")
}

#[test]
fn fetch_stream_helpers_materialize_independently_in_child_and_isolated_realms() {
    let mut vm = new_storage_test_vm("https://fetch-helper-realms.test/");
    vm.eval(
        r#"(() => {
            const frame = document.createElement('iframe');
            (document.body || document.documentElement || document).appendChild(frame);
            void frame.contentWindow;
        })()"#,
    )
    .expect("create child frame");
    let child_id = materialize_single_child_default_realm_for_test(&mut vm, "Fetch helpers");
    let child_ptr = vm
        .child_frame_realm_context_ptr_for_execution_context_id(child_id)
        .expect("child context");
    let frame_id = vm
        .live_child_default_runtime_realm_inventory()
        .into_iter()
        .find(|realm| realm.context_id == child_id)
        .and_then(|realm| realm.frame_id)
        .expect("child frame ID");
    let isolated_id = vm
        .create_isolated_world_for_frame(&frame_id, "fetch-helper-isolated", false)
        .expect("isolated child realm");
    let isolated_ptr = &vm
        .page_isolated_world_contexts
        .context(isolated_id)
        .expect("isolated context")
        .context as *const v8::Global<v8::Context>;
    assert!(default_fetch_helpers(&mut vm).is_none());
    assert!(fetch_helpers_in_context(&mut vm, child_ptr).is_none());
    assert!(fetch_helpers_in_context(&mut vm, isolated_ptr).is_none());

    const CLONE: &str = "String(new Response(new ReadableStream({start(c) {c.close()}})).clone() instanceof Response)";
    assert_eq!(
        vm.eval_in_child_default_context(child_id, CLONE).unwrap(),
        "true"
    );
    let child_helpers = fetch_helpers_in_context(&mut vm, child_ptr).expect("child helpers");
    assert!(default_fetch_helpers(&mut vm).is_none());
    assert!(fetch_helpers_in_context(&mut vm, isolated_ptr).is_none());

    assert_eq!(
        vm.eval_in_isolated_context(isolated_id, CLONE).unwrap(),
        "true"
    );
    vm.with_context_scope_by_ptr_and_checkpoint_for_test(isolated_ptr, |scope, _| {
        let child_helpers = v8::Local::new(scope, &child_helpers);
        assert!(
            !fetch_helpers_in_scope(scope)
                .unwrap()
                .strict_equals(child_helpers)
        );
        Ok(())
    })
    .expect("child and isolated helpers must not share function realms");
    assert!(default_fetch_helpers(&mut vm).is_none());
}

#[test]
fn fetch_stream_helpers_work_when_string_code_generation_is_disallowed() {
    for policy in ["script-src 'self'", "require-trusted-types-for 'script'"] {
        let mut vm = new_storage_test_vm("https://fetch-helper-no-eval.test/");
        // Window contexts already disable V8's unconditional string-codegen
        // permission; Moli's callback then consults the document's CSP/TT policy.
        vm.set_response_content_security_policies(&[policy.to_owned()]);
        const AUTHOR_EVAL: &str =
            "(() => { try { eval('1 + 1'); return 'allowed'; } catch (e) { return e.name; } })()";
        assert_eq!(vm.eval(AUTHOR_EVAL).unwrap(), "EvalError", "{policy}");
        assert!(default_fetch_helpers(&mut vm).is_none());
        assert_eq!(
            vm.eval("String(new Response(new ReadableStream()).clone() instanceof Response)")
                .expect("native Fetch helper must not require author eval permission"),
            "true",
            "{policy}"
        );
        assert!(default_fetch_helpers(&mut vm).is_some());
        assert_eq!(
            vm.eval(AUTHOR_EVAL).expect("author eval remains disabled"),
            "EvalError",
            "{policy}"
        );
    }
}

#[test]
fn fetch_stream_helpers_are_lazy_private_and_cached_in_the_window_realm() {
    let mut vm = new_storage_test_vm("https://lazy-fetch-helpers.test/");
    assert!(default_fetch_helpers(&mut vm).is_none());
    assert_eq!(
        vm.eval(
            r#"JSON.stringify([
                typeof Request, typeof Response, typeof ReadableStream,
                Object.hasOwn(globalThis, '__lmConsumeReadableStreamBody'),
                Object.hasOwn(globalThis, '__lmTeeReadableStreamBody'),
                Object.hasOwn(globalThis, '__moliFetchBodyStreamHelpers')
            ])"#,
        )
        .expect("Fetch constructors do not compile stream helpers"),
        r#"["function","function","function",false,false,false]"#
    );
    assert!(default_fetch_helpers(&mut vm).is_none());

    vm.eval("new Response(new ReadableStream()).clone()")
        .expect("cloning a JS stream initializes its realm's helpers");
    let first = default_fetch_helpers(&mut vm).expect("initialized Fetch helpers");
    vm.eval("new Response(new ReadableStream()).clone()")
        .expect("another clone uses cached helpers");
    vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
        let first = v8::Local::new(scope, &first);
        assert!(fetch_helpers_in_scope(scope).unwrap().strict_equals(first));
        Ok(())
    })
    .expect("Fetch helper cache identity");
}

#[test]
fn fetch_stream_helpers_do_not_resolve_author_visible_internal_names() {
    let mut vm = new_storage_test_vm("https://private-fetch-helpers.test/");
    assert!(default_fetch_helpers(&mut vm).is_none());
    let messages = vm
        .dispatch_inspector_protocol_message(
            &serde_json::json!({
                "id": 921,
                "method": "Runtime.evaluate",
                "params": {
                    "awaitPromise": true,
                    "returnByValue": true,
                    "expression": r#"(async () => {
                        let reads = 0;
                        for (const name of ['__lmConsumeReadableStreamBody', '__lmTeeReadableStreamBody', '__moliFetchBodyStreamHelpers']) {
                            Object.defineProperty(globalThis, name, {
                                get() { reads++; throw new Error('author slot read'); }
                            });
                        }
                        const body = () => new ReadableStream({
                            start(controller) {
                                controller.enqueue(new Uint8Array([111, 107]));
                                controller.close();
                            }
                        });
                        // Consumer is the first helper needed in this realm.
                        const first = await new Response(body()).text();
                        const response = new Response(body());
                        const clone = response.clone();
                        return [first, ...await Promise.all([response.text(), clone.text()]), reads];
                    })()"#
                }
            })
            .to_string(),
        )
        .expect("Fetch helper promise dispatch");
    // The chain contains only ready stream data/microtasks. Observe the exact
    // Inspector response, not sleeps or a fixed number of dummy eval turns.
    let response = messages
        .iter()
        .find(|message| message["id"] == 921)
        .unwrap_or_else(|| panic!("Fetch helper promise did not settle: {messages:#?}"));
    assert!(response.get("error").is_none(), "{response:#?}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response:#?}"
    );
    assert_eq!(
        response["result"]["result"]["value"],
        serde_json::json!(["ok", "ok", "ok", 0])
    );
    assert!(default_fetch_helpers(&mut vm).is_some());
}
